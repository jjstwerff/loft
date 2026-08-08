<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 24 — `#c`: call a C library with no Rust in it

## Status

**Done — every arc.** The architecture probe is built and has run
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

**Arc B closes the divergence arc C opened.** The interpreter resolves the
symbol and calls it through the fixed 0..=12 trampoline ladder, and the two
backends now produce **byte-identical output** on the same program. It needed no
codegen change at all: a body-less definition with no `#native` symbol already
resolves through `library_names` under its own name, so registering the binding
there is what routes the call.

The return is the one place the interpreter differs from `--native` in kind, and
it is where the declaration earns its keep twice over. rustc gets the ABI right
from the typed extern; the interpreter reads a **raw `u64` register**, so
`atoi("-1")` is 4294967295 until the declared width truncates and re-extends it.
`narrow_return` is that one step, and its unit test pins the raw register value
so the failure mode is written down rather than described.

**What arc B refused, and why it is refused on BOTH backends.** A `char *`
return is a real C shape and not yet buildable: a loft `text` return crosses
through the destination-passing convention (`is_text_dest_native`), which
neither caller is wired into. Arc C had written a half version of it that no
test covered — arc B's first attempt at the same shape SIGSEGV'd the interpreter
while rustc rejected the native side, two different failures for one gap. It is
now refused at the DECLARATION, so both backends say the same words, and it is
arc D's to build. That is the pattern this plan keeps landing on: a shape that
works on one backend only is worse than a shape that works on neither.

**Arc D, second slice: the `char *` return, built.** The refusal is gone and the
shape works on both backends, in every value position. What made it small was
reading the emission before writing any of it: the parser ALREADY normalises a
text-producing call in argument, operand or element position into a synthetic
local assignment (`wrap_value_text_dest` → `{#synth text dest}`), so there is
exactly ONE emission site to teach rather than one per position. A `#c` text
binding is then a third member of an existing class — body-less, external, no
`_dest` sibling and no work buffer of its own — and reuses the cdylib route
(`n_set_bridge_dest` + the call) unchanged. The whole interpreter half is one
predicate (`is_c_text_call`) read at three sites, and `--native` is one clause in
`returns_owned_string` plus the copy in the emitted body.

The design cost sat entirely in **three questions C's type system cannot
answer**, which the plan had left to the fixture — and the fixture had already
written both halves of the first one down:

1. **Who frees it.** `strerror` and `PQerrorMessage` hand back storage the caller
   must not free; `strdup` hands back storage it must. `const` does NOT separate
   them (POSIX spells both `char *`), so a rule read off the signature would free
   static memory on a wrong guess. A leak is recoverable and that is not, so
   **loft never frees**, and caller-frees goes through the shim — which is the
   plan's own routing, now measured rather than assumed.
2. **Where the bytes end.** At the first NUL, because that is what `char *` means.
   loft text carries a length and may hold an interior NUL; the crossing
   truncates rather than inventing one.
3. **NULL, and bytes that are not UTF-8.** NULL is loft null (a CONTENT sentinel,
   so a dest record carries it); invalid UTF-8 is replaced, not refused — a
   locale-encoded byte from a C library must not take the program down (C80).

**And the boundary probe found a real defect in the arc-A check**, which is the
whole reason to probe the edge rather than the happy path: `-> text?` was
*refused*, with a message written for the ARGUMENT direction ("declare the
parameter non-null"). But the two directions are not symmetric. loft can hand C
no value for a null, so a nullable ARGUMENT is genuinely unrepresentable — while
C's NULL *return* is exactly "no string", and `text?` is the one spelling that
lets the null-flow analysis SEE the null the crossing already carries. Declared
`text`, that same NULL still arrives, silently, as the content sentinel. So
`text?` is now accepted for a `char *` return and is the recommended spelling;
everything else `Optional` stays refused.

Two cells the fixture could not express before, added to it: a symbol that
returns NULL conditionally (`lc_maybe_text`), and one that returns Latin-1 bytes
(`lc_latin1_text`). Both are ordinary C, and both are places the two backends
could have drifted apart without either looking wrong.

The pointee spelling earns one decision here, having been demoted to
diagnostics-only by arc C: a pointer return bound to `text` must be spelled
`char*`. `void*` bound to `text` is either a mistake or a handle that wanted
`integer`, and — the plan's premise again — nothing at runtime tells them apart.

**Arc D, first slice: a binding reaches a real library.** `[c] libs` in a
package manifest is `dlopen`ed by the interpreter and linked by `--native` — one
declaration, both halves. With it the fixture matrix runs, and it is what makes
the mechanism testable at all: arcs A–C could only reach libc, which is already
in the process.

**The matrix is green on both backends, byte-identical**
(`c_binding_matrix_against_a_declared_library`): every integer width, a text
argument, a vector as pointer + count, the opaque-handle open/read/bump/close
cycle, and the 7-argument call that straddles the SysV register/stack boundary.

Three defects the fixture found that libc could not, all of them *other code
claiming a `#c` definition*:

1. **The handle convention was half-built.** A pointer RETURN could be held as a
   loft `integer`, but that integer could not be passed back as a pointer
   ARGUMENT — so `lc_open` bound and `lc_read` did not. loft has no type that
   distinguishes a handle from an integer (which is why the plan chose `integer`
   for it), so the check now allows a C pointer wherever a loft scalar sits, in
   both directions.
2. **The auto-native driver claimed every `#c` definition.** A body-less
   declaration looks native-compilable, so a package of `#c` bindings had loft
   generate a Rust cdylib exporting `loft_shared_*` bridges for symbols that
   live in C, overwrite `def.native`, and warn "calling it will panic" at every
   call. A `#c` definition is already bound; `native_gate` now says so.
3. **The script classifier read a `#c` library as a beginner script.** It
   consumed one string per annotation, and `#c` takes two — so the signature
   looked like a loose top-level statement. Annotations may now carry more than
   one argument.

None of these were in the design. All three are the same shape as arc C's
"inert wasn't inert": a new kind of definition has to be excluded from every
path that pattern-matches on *body-less*, and the paths do not announce
themselves. Arc E should expect more of them.

One thing arc A learned the hard way, worth carrying into B: the baked IR field
offsets in `data_store.rs` are a MIRROR of the registered schema, and the schema
packs by size rather than declaration order — so two new `text` fields moved
every trailing boolean and the stride. Hand-guessed offsets segfaulted the
round-trip test; `baked_layout_mirrors_loft_schema` is the instrument that names
the real ones, and it must be read *before* the constants are written, not
after.

**Arc D, third slice: the shim builds, so the trade is real.** Every signature
the fixed trampolines cannot express routes to an ANSI-C shim — that is the
plan's whole complexity trade, and this session widened the dependency by
sending caller-frees `char *` returns there too. Until now loft could not build
one, which made the escape hatch a claim: an author's actual alternative was the
rustc toolchain `#c` exists to avoid.

`[c] shim = "src/shim.c"` is compiled with `cc` and the result is registered
**exactly like a `[c] libs` entry**. That is the design decision worth naming:
nothing downstream can tell a shim from any other C library, so the `dlopen`,
the `--native` link line and the symbol resolver stay ONE code path. The plan's
counted risk is `N × silence` — a fact restated at several sites goes wrong
quietly at each — and a shim registered as its own kind of thing would have
added a fourth site to every one of them. Registering both through one helper
also collapsed the two copies the `[c] libs` loop already had.

The artifact is content-addressed by the sources plus the compiler's identity,
for the reason loft#715 made the Rust cdylibs content-addressed: an edit
produces a different file rather than racing to overwrite one another process is
reading, and a toolchain change rebuilds rather than reusing a stale ABI.

`loft install` builds it too — not to make it work (the parser would), but to
move WHEN the answer arrives: a package needing a C compiler should say so while
the user is installing packages, not inside the first run of their program.

The fixture is `pkg/lcshim/`, one cell per shape that needs a shim: a `double`
argument, an out-parameter, and a caller-frees `char *`. The float cell is
hand-computed — 2.5 × 4.0 is exactly 10.0 — so a shim returning a plausible
wrong double fails rather than agreeing with itself.

**Arc F is closed by @PLN23 S1**, on a real system library rather than the
fixture: `mysql_get_client_info()` and `mysql_get_client_version()` bound against
`libmariadb.so.3`, both backends byte-identical, zero rustc and no dev headers
(`a_c_binding_reaches_a_versioned_system_library_on_both_backends`). What it
proved that the fixture could not is the VERSIONED soname: `-l dylib=mariadb`
sends the linker to `libmariadb.so`, the `-dev` symlink, while the interpreter
`dlopen`s `libmariadb.so.3`, the runtime file — one declaration resolving to two
different files, so the program ran interpreted and failed to LINK on a machine
where the library is plainly installed. `-l:<file>` is the fix, and it is the
same defect arc G's design later had to keep from re-opening one level up.

**Arc E — the answer is a refusal, and the measurement is why.** The plan wrote
wasm off as having "no C ABI to bind to at all". It has one. `wasm32-wasip2`
links a libc, so a binding to `strlen` resolved, LINKED with a warning, and then
trapped at the call — `signature_mismatch: strlen`, `(i32) -> i64` against the
sysroot's `(i32) -> i32` — because wasm32 is a THIRD data model (ILP32: `long`,
`size_t` and every pointer are 32 bits) and the extern carried the host's widths.
That is the plan's own `N × silence` arriving at a site nobody had counted:
`CTarget::host()` is read at every emission site, and one of the targets is not
the host.

The three ways it used to end, all on one tree, none of them distinguishable by
a reader from a bug in their own program:

| cell | before |
|---|---|
| `--native-wasm`, reachable `#c`, symbol IS in the WASI sysroot | **builds** (linker warning) and **traps at the call** |
| `--native-wasm`, reachable `#c`, symbol is not | `rust-lld: undefined symbol: lc_strlen` — names neither package nor library |
| `--native-wasm`, a package declaring `[c] optional-libs` | `E0433: cannot find c_call in loft`, once per symbol — **for bindings the program never called** |
| `--html`, reachable `#c` | already refused by name (the one column that was defined) |
| `--html`, `c_library_available("…")` | `E0425: cannot find value __C_LIB_SYMS` |

The last row is the one that decided the shape of the fix. `c_library_available`
is the query a `#c` library is *told* to ask before calling into an optional
backend — and it did not compile on the target where every C library is missing,
because its tables were emitted only on non-browser targets. A refusal that
names a cure has to leave the cure reachable.

So: **no `#c` emission on either wasm target** — no `extern "C"`, no lazy
resolver — and the refusal at the CALL, which makes it reachability-scoped for
free. A library may declare `#c` bindings and still build for wasm as long as
the wasm program does not reach one; that was already true for `#native`
(@PLN26 / P269) and it is what keeps a multi-target library from having to fork
its source. The tables move OUT of the target gate and are emitted everywhere.

**Why the refusal rather than the static-C route**, which the plan listed first.
Making the sysroot-libc cell *work* is possible — it is a `CTarget` for ILP32 —
and it would be the worst of the three answers: a `#c` symbol would resolve on
wasm if and only if it happened to live in the WASI sysroot, a rule invisible
from the declaration and different on every toolchain, while every real `#c`
library (which binds a SYSTEM library, not libc) still failed. Compiling a
package's own `[c] shim` to wasm with `clang --target=wasm32-wasi` remains the
honest affirmative route and is **not built**: it needs a C cross-compiler in the
build environment (this host has none, so nothing about it could be proven here),
and it covers only a shim that is pure computation — never a database client,
whose capability does not exist in a browser at all. It changes the refusal into
a build step; it does not change the refusal's shape. The manifest key it needs
lands with it, because a key with one legal value is not a choice.

**Open question 3 — answered, and it was a hole.** A `#c` binding is gated by
the sandbox's `native_ffi`, exactly like an external `#native` bridge, and never
by a `#cap` grant. Measured before it was closed: a sandboxed script reaching a
`#c` binding tagged `db#read`, under a profile granting `db#read` and leaving
`native_ffi` at its default false, was **admitted and ran the C**. The cause is
this plan's recurring shape seen from the other side — arc D's three defects were
paths that pattern-matched on *body-less* and wrongly CLAIMED a `#c` definition;
this is a path that pattern-matches on `#native` and therefore silently ADMITTED
one. Both the FFI ban and `reachable_ffi_bridges` key on `def.native()`, which
arc A leaves empty on purpose so the Rust dispatch path cannot take a `#c` def.

The rule, stated so it does not have to be re-derived: **a capability grant says
what DATA a script may touch; it cannot say "and arbitrary machine code may run
in this process".** That is the line `native_ffi` draws, and `#c` is the stronger
case of the same surface — a Rust bridge at least has a marshalling layer, and a
`#c` call has none. An allow-listed library still admits its bindings, unchanged:
that is the host vetting the library as a unit, the answer `#native` bridges
already get.

## Goal

A loft library binds directly to a system C library — **no rustc anywhere in
the library**, no libffi — and reaches the same standard of proof as a Rust
`#native` library: the same four-target matrix, the same signature checking,
the same failure reporting, the same capability declarations.

- **Effort:** M for arcs A–D (the native + interpreter halves). Arc E (the other
  two targets) came in far smaller than budgeted, because the honest answer there
  is a refusal rather than a port — see the status section.
- **Design:** ✓ — invariant named, claims probed, every open question answered.

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
| `--native-wasm` | cross-compiled wasm rlib | **refused at the call, by name** | differs — a wasm module cannot `dlopen` |
| `--html` | hand-built `[wasm.bridge]` crate | **refused at the call, by name** | same, and `#native` needs the hand-built bridge anyway |
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

## Arc G — optional libraries: binding more than the user installs

@PLN23's client binds three C libraries (sqlite3, mariadb, libpq). A package
that declares all three today REQUIRES all three, so "supports three databases"
reads to a user as "install three database clients to get one". The question is
whether a declared library can be loaded on demand, and the answer is **not
symmetric across the backends** — which is the finding, because it means this
is not the interpreter change it looks like.

**Measured on this tree** (2026-08-02), not reasoned from the code. The probe is
`tests/fixtures/c_abi/pkg/lcabi` with one bogus soname appended to its `[c]
libs` and one binding declared against it: a package naming one present library
and one absent one, with the absent one's binding called in only the second
cell. The present library's call (`lc_strlen("hello")` → **5**, hand-computed)
is in every cell, so a cell that prints nothing is a broken harness rather than
a result:

| | `--interpret` | `--native` |
|---|---|---|
| absent library, its binding **never called** | runs, right answer | **does not build** — `unable to find library -l:libtotallynotreal.so.9` |
| absent library, its binding **called** | right answer, then a Rust panic at the call | does not build |

Two facts, neither of them what the code reads like:

- **The interpreter is already lazy for an ABSENT library, and nobody designed
  it that way.** `c_call::register` calls `load_c_library` and discards the
  result (`src/c_call.rs:259`), and `c_call::dispatch` resolves the symbol PER
  CALL (`src/c_call.rs:360`) rather than caching a pointer at wiring time — so
  `CBinding` holds no pointer and a failed `dlopen` costs nothing until a
  lookup. The eager loop is a startup COST for libraries that exist, not a
  barrier to ones that do not.
- **`--native` fails earlier than "won't start": it won't BUILD.** Every
  declared library goes on the link line (`add_c_library_flags`,
  `src/native_utils.rs:1471`), so an absent one is a link error on a program
  that never calls into it. `--native` is the default backend, so that link
  line is the whole of the gap.

### The invariant this arc must not break

Arc D already paid for one half of this: `-lmariadb` resolves to the `-dev`
package's symlink while the interpreter `dlopen`s `libmariadb.so.3`, so one
declaration named two different files, and `-l:<file>` was the fix. Making the
interpreter lazier while `--native` stays eager re-opens that seam one level up
— the same manifest would run under `--interpret` and refuse to build under
`--native`, on a machine where nothing is wrong. **Laziness is a two-backend
change or it is not taken.** The asymmetry in the table above is not a
precedent for shipping another one; it is the defect.

### Design

1. **`[c] optional-libs = "libpq.so.5, libmariadb.so.3"` — a second list, not a
   flip of the first.** `[c] libs` keeps meaning *this package does not work
   without it*, which is worth an early and actionable failure. Optionality is a
   claim the author makes about their own library, and the manifest is where
   they make it — deriving it from whether a `dlopen` happened to succeed would
   make the same package required on one machine and optional on the next.
2. **Interpreter:** drop optional entries from `register`'s eager loop and load
   them from `resolve`'s MISS path — loaded cdylibs → the process → `dlopen` the
   declared-but-unopened optional libraries → retry. Arc B's rule survives
   intact: one resolver, so a symbol cannot mean two different things depending
   on which caller asked.
3. **`--native`:** no `-l` flag for an optional entry, and
   `output_c_direct_call` emits the call through a per-symbol lazily-resolved
   pointer (`dlopen` the declared soname + `dlsym` on first use, memoised)
   instead of a plain `extern "C"`. **The resolved pointer must be transmuted to
   the typed signature `CSignature` already produces.** A `u64` trampoline here
   would throw away the one thing arc C bought — rustc getting the ABI right
   from the declared widths — and hand back the exact wrong-width return arc C
   measured (`atoi("-1")` read as 4294967295).
4. **The availability query, without which "optional" only moves the failure
   later.** A missing symbol panics (`src/c_call.rs:361`) — and it arrives as a
   Rust panic naming a `src/` path, not as a loft diagnostic. That is the right
   answer for a library the manifest PROMISED. It is the wrong answer for one it
   merely offered, because the point of the arc is that the library CHOOSES a
   backend, and choosing means asking first. Lean a builtin taking the declared
   soname (`c_library_available("libpq.so.5")`) over per-binding null returns: it
   answers before the call rather than at it, and it does not put a `τ?` on
   every binding in an optional library.
5. **The query answers about SYMBOLS, not about the file.** A present library of
   the wrong vintage exports a subset, so a file-granular answer is available
   where the call still panics — the version-skew hole, and the one place a
   naive implementation of (4) is worse than no query at all. `available` is
   therefore *the `dlopen` succeeded AND every `#c` symbol declared against this
   library resolves in it*. Both are already computable at the moment the query
   runs, and the loft declarations are the list to check against, which is this
   plan's invariant doing its job.

### Re-assertion sites — counted, before any code

Optional-vs-required is a new fact, restated by (1) the interpreter's load
decision, (2) the native link line, (3) the native emission (`extern "C"` vs
lazy pointer), and (4) the availability query. Four sites, and this plan's
counted risk is that omission is **silent at every one**. The collapse is the
one the plan already uses for signatures: parse the manifest once into a flag on
the `data.c_libraries` entry, and have all four read that. Nothing re-derives
optionality from a name, a path shape, or a `dlopen` outcome.

### What would falsify it

- **A lazily-resolved `--native` call is too slow** for a driver in its inner
  loop (one predictable branch per call, against a direct `extern "C"`). Measure
  on arc F's libpq subset before making it the emission for optional entries; if
  it bites, the answer is one resolved-once table per library, not abandoning
  the arc.
- **`dlopen` from a generated binary needs a link flag on some target** (`-ldl`
  historically), which would put a flag back on the line this arc exists to
  empty. Check against the target list arc E settles, not just this host.
  (Moot on the wasm targets themselves: no `#c` call is emitted there at all.)
- **The `--interpret` cell above is a bug, not a feature.** A required library
  that is absent should arguably fail at load with the package named, and today
  it does not — the discarded `load_c_library` result. If tightening that is
  right, it lands with this arc rather than against it, because the two halves
  are one decision: required fails early, optional does not fail at all.

Rust cdylibs (`#native`) are deliberately **out of scope**. They could go lazy
too — `compute_sig` derives the signature from the loft declarations alone and
`native_auto_dispatch` already does a per-call table lookup — but the eager step
there is `auto_build_native`, a cargo BUILD, and deferring it moves when compile
errors surface. Different fact, different plan (@PLN21/@PLN11).

### What the build corrected

Two of the design's claims came back changed, and the second is the one worth
carrying into any future work on this plan.

1. **The attribution rule was wrong in the direction that mattered.** The design
   said a package declaring several libraries lists its symbols under each, and
   called the result "conservative — it can only say no where a call would have
   worked." Built, that is not conservative, it is useless: a package binding
   sqlite AND duckdb reports **sqlite** unavailable because a duckdb symbol is
   missing, which is exactly the arrangement optional libraries exist for. So a
   symbol is attributable only when its library is the only OPTIONAL one its
   package declares.

   The word "optional" in that rule was itself a correction. Counting *every*
   declared library switched skew detection off for the duckdb backend written
   to demonstrate it — because a `#c` package almost always ships a `[c] shim`,
   which registers as an ordinary library and so read as a competing one. A
   required entry can never be why a symbol is missing (the package does not
   load without it), and a shim is present by construction because loft just
   built it. Only another OPTIONAL library makes attribution ambiguous.

2. **Process-global state does not reach a package cdylib.** The runtime tables
   were static, which is correct in the interpreter and in a `--native` binary
   and WRONG in the third configuration nobody had listed: an auto-built package
   cdylib links its own copy of loft, so it has its own copy of the statics. The
   symptom was as clean as it gets — `duckdb_available()` answered **false** from
   inside the package while the identical call from the program answered
   **true**, on the same run. The generated source carries the tables and now
   passes them in (`library_available_native`), so the answer no longer depends
   on which linkage unit asked.

   The lesson generalises past this arc: **"both backends" is not the same as
   "every linkage unit."** The `#c` calls themselves were fine here only because
   the generator already emits its tables into the cdylib; a fact reached through
   a `#rust` body has three homes to be right in, not two.

## Sub-arcs

| Item | Status |
|---|---|
| **A** — `#c "sym" "<signature>"`: parser, the `CSignature` type, the compile-time check | **Done** — `src/c_signature.rs`, inert; widths resolve per target |
| **B** — the interpreter caller: `dlsym` + the per-arity trampolines + return-width dispatch | **Done** — `src/c_call.rs`; both backends answer identically |
| **C** — the `--native` caller: emit the typed `extern "C"` decl from `CSignature` | **Done** — loft calls libc directly, no rustc in any library |
| **D** — packaging | **Done** — `[c] libs` declares + loads + links, a `char *` return crosses as `text` / `text?`, and `[c] shim` is ANSI-C loft compiles itself (`cc`, never rustc) at parse and at `loft install` |
| **E** — the other two targets (the parity arc) | **Done** — both wasm shapes refuse a REACHABLE `#c` call by name (function, symbol, package, target); an unused declaration still builds; `c_library_available` compiles and answers `false` there |
| **F** — prove it: a real system library for @PLN23, zero rustc | **Done** — @PLN23 S1: `libmariadb.so.3` through a versioned soname, both backends identical |
| **G** — optional libraries: `[c] optional-libs` loaded on demand, both backends, plus the availability query | **Done** — `c_library_available`, a duckdb backend in `tests/fixtures/sqldb/duckdb/`, both backends byte-identical present AND absent |

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
5. **G after F**, for the same reason D came before F: the three-backend client
   is what says whether "optional" is worth a manifest key or whether a user
   installing one client library was never actually the complaint.
6. **E** last, and only against a real consumer's actual target list.

## Composition matrix — Stage A

**Built, and it ran before the design was written.**
[`tests/fixtures/c_abi/`](../../../../tests/fixtures/c_abi/) is the matrix: one
C function per loft type (every integer width, boundaries at the null sentinel,
`boolean`'s three states, `text` with an interior NUL, three vector shapes,
opaque handles, arities straddling the register/stack split) with hand-computed
expectations and a C self-test that validates the oracle before any loft is
involved. Arc B is done when every cell is green on both backends.

## Open design questions — all answered

1. ~~Does the C integer width need spelling in the declaration?~~ **Settled by
   measurement: yes, at least for returns.** Whether arguments also need it is a
   narrower question — the probe says they cross correctly regardless, so the
   spelling may be *required for the return, checked for the arguments*.
2. ~~**Arc E — what does a `#c` library do on wasm and in the browser?**~~
   **Answered: it is refused, by name, at the call.** Of the three routes the
   question listed, out-of-process (@PLN119) and a wasm implementation of the
   library are the ones that work, and both are the author's to take; the
   static-`clang` route is real but unbuilt (no C cross-compiler was available to
   prove a single cell, and it reaches only a pure-computation shim). See the
   status section for the measurement that killed the fourth option — making the
   sysroot-libc case work — which would have shipped a capability whose
   availability no author could predict.
3. ~~**Does a `#c` symbol need an effect declaration to be admissible?**~~
   **Answered: no — effects are the wrong instrument, `native_ffi` is the right
   one.** The question assumed the choice was between trusting the declared
   effect and refusing `#c` under `--sandbox` outright. It is neither: the
   sandbox already separates *what data may be touched* (`#cap`) from *may
   foreign code run at all* (`native_ffi`), and a `#c` binding belongs on the
   second axis. It had fallen onto the first, so a granted `db#read` admitted an
   arbitrary C call — see the status section. A `#c` symbol may still carry a
   `#cap` tag and it is still checked; it is simply never sufficient.

## Cross-arc dependencies

- **@PLN23** (MariaDB + PostgreSQL clients) — the first consumer and the reason
  the fixture carries the opaque-handle shape. Blocked on this, and the reason
  arc G exists: it binds three C libraries, and without G a user gets one
  database by installing three.
- **@PLN119** (out-of-process libraries) — arc E's answer for any `#c` library
  that has to be reachable from wasm at all; the refusal names it.
- **@PLN21** (prebuilt native libs) — a `#c` library needs no rustc prebuild,
  only the system lib and the optional `cc` shim; the manifest keys are shared.

## See also

- [LIBRARY_NAMING.md](LIBRARY_NAMING.md) — **design, not built.** A `[c]` library
  named ONCE by identity (`sqlite3@0`) with every filename, link flag and search
  path derived from it, so no step ever parses a spelling back out of a filename.
  Written off four measured failures, including the `-l` stem that stripped
  `.so`/`.dylib` but not `.dll`.
- [PACKAGES.md § Direct C binding](../../PACKAGES.md) — the reference home; this
  plan's outcome lands there.
- [NATIVE.md](../../NATIVE.md) — the Rust path this is measured against.
- [`tests/fixtures/c_abi/`](../../../../tests/fixtures/c_abi/) — the fixture and
  the probe, with the measured findings.
- [@PLN24](https://github.com/loft-lang/plans/issues/24) — the tracker issue.
