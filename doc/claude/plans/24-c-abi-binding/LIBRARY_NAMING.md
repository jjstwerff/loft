<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 24 — a C library is named ONCE, by identity

How any loft package that binds ANSI-C names its libraries, so that one
declaration works on Linux, macOS and Windows without the author writing a
filename for a platform they may never have used.

**Design only. Nothing here is built.**

## The problem, measured

Today a manifest names a library by its **Linux ELF filename**:

```toml
[c]
optional-libs = "libsqlite3.so.0, libpq.so.5, libmariadb.so.3"
shim = "src/shim.c"
```

Everything else is recovered from that string by surgery at each point of use.
Four failures came out of that arrangement, and none of them is a typo:

- **The `-l` stem is derived by REMOVING decoration.** `add_c_library_flags`
  builds the link stem by stripping `lib` and everything from `.so` / `.dylib`.
  `.dll` was not in the chain, so Windows sent `-l dylib=sqlite_shim_<key>.dll`
  and MSVC opened `sqlite_shim_<key>.dll.lib` — a name nothing had ever written.
  The bug survived because the stem is reconstructed from a filename loft itself
  decorated.
- **A caller that asks for the declared name asks about Linux.** The sqldb test
  gated each backend on `libsqlite3.so.0` existing; on macOS `/usr/lib` holds
  `libsqlite3.dylib`, so every conditional cell skipped in silence and the suite
  passed on macOS for months having opened no database
  ([@PLN23 PLATFORMS.md](../23-db-clients/PLATFORMS.md)).
- **The version has to be MOVED, not appended.** `libmariadb.so.3` is
  `libmariadb.3.dylib` on macOS — the soversion crosses the extension — and on
  Windows both the `lib` prefix and the version usually disappear. That is a
  per-OS rule currently expressed as `split_once(".so")` plus reassembly.
- **A symbol cannot say which library it came from.** `#c "sqlite3_open" "…"`
  names no library, so every `#c` symbol in a package is attributed to whichever
  library it declared. A package that declares a library AND builds a shim has
  both sets attributed to one soname, which is why
  `c_library_available("libsqlite3.so.0")` answers **false** on a machine where
  every sqlite call works.

The common cause: **the filename is treated as the primary fact and the identity
is recovered from it.** Every consumer re-derives, and each derivation is a place
to be wrong in silence.

## The invariant

> **A `[c]` library is named once, by IDENTITY. Every filename, link flag,
> search path and staged artifact is DERIVED from that identity and the target —
> and nothing is ever parsed back out of a filename.**

The second clause is the load-bearing one. It is what makes the `-l` stem free
rather than recovered, and it deletes that whole class of bug instead of fixing
one instance.

## The model

An identity is a **name** and, when the library has a stable ABI generation, a
**version**:

```toml
[c]
libs = ["sqlite3@0", "pq@5", "mariadb@3"]
optional-libs = ["duckdb"]
shim = "src/shim.c"
```

`sqlite3@0` is *the C library whose SONAME generation is 0* — not a file. The
author writes what they would say out loud; loft writes the filenames.

### The derivation, one table

| target | candidates, in order |
|---|---|
| Linux / ELF | `lib{name}.so.{v}`, `lib{name}.so` |
| macOS | `lib{name}.{v}.dylib`, `lib{name}.dylib` |
| Windows | `{name}.dll`, `lib{name}.dll`, `{name}-{v}.dll`, `lib{name}-{v}.dll` |
| link flag (all) | `-l {name}` |
| wasm / browser | **none** — a defined refusal, see below |

The versioned form is tried first everywhere because an ABI generation is the
thing the declaration actually promised; the bare form is the fallback for a
development symlink. Windows carries four candidates because the convention is
genuinely not settled there — MSVC-built DLLs drop the `lib` prefix, MinGW keeps
it, and OpenSSL-style projects append the version to the stem.

**The link flag needs no computation at all.** That is the point: `-l {name}`
comes straight from the identity, so there is no stripping step and therefore no
`.dll`-shaped hole in it.

### The escape hatch, because reality is not uniform

Some libraries genuinely are not one name. `libcrypto.so.3` ships on Windows as
`libcrypto-3-x64.dll`. A per-target override is a declaration, not surgery:

```toml
[c.libs.crypto]
version = 3
windows  = "libcrypto-3-x64"     # stem only; the .dll is still derived
```

An override replaces the *stem*, never the extension or the search — so it
cannot reintroduce a filename that some later step has to parse.

## Backwards compatibility, which is absolute

Published packages already declare `optional-libs = "libsqlite3.so.0"`, and at
contract 1 no functioning program ever breaks ([COMPATIBILITY.md](../../COMPATIBILITY.md)).
So the old spelling stays valid **and is defined in terms of the new one**: a
declaration matching `lib{name}.so{.v}?` parses to the identity `{name}@{v}`. It
is the same value by a longer route, not a second mechanism — which matters,
because a second mechanism is what this design exists to remove.

A declaration that does NOT match that shape (an absolute path, a vendored
oddity) keeps today's literal-filename behaviour, and `loft check` advises the
identity form. Nothing is rejected.

## Re-assertion sites, counted before any code

Every one of these derives a spelling today, independently:

1. manifest parse (`manifest.rs`)
2. interpreter load (`extensions::load_c_library`)
3. `--native` link line (`native_utils::add_c_library_flags`)
4. availability query (`c_call::library_available`)
5. shim build + artifact naming (`c_shim`, `native_lib::platform_cdylib_name`)
6. Windows DLL staging (`native_utils::stage_native_dlls`)
7. the wasm / `--html` targets, which must refuse
8. `loft install` / the registry, when a package ships a vendored copy

**Eight sites, and omission is silent at every one** — a wrong spelling is a
library that "is not installed". The design collapses them to a single resolver,
`platform::CLib::candidates(target)`, which the other seven consult and none
re-derives. `platform::lib_variants` is already two-thirds of it; this makes it
the only one and gives it the identity to work from instead of a filename.

## The claim that can be falsified

The invariant implies a property that is machine-checkable, needs no C library
present, and **fails today**:

> For every identity `id` and every target `t`, the link stem loft emits for
> `candidates(id, t)` equals `id.name`.

A conformance test over the cross product of {every `[c]` declaration in the tree
and the registry} × {Linux, macOS, Windows} asserts it. Run against today's code
the Windows column fails on exactly the `.dll` case that shipped — so the test is
proven to fire before the change that makes it pass, which is the whole reason to
write it first.

A second property, equally cheap: **`candidates` is injective on stems** — two
different identities never produce the same file candidate. That is what would
catch a future `pq@5` / `libpq@5` ambiguity.

## Symbol attribution — the part that needs a language change

The resolver above fixes naming. It does **not** fix failure four, because that
one is not about spelling: `#c "sqlite3_open" "sig"` carries no library, so loft
cannot tell a shim symbol from a sqlite symbol.

Three options, and the cost is not equal:

- **A third string** — `#c "sqlite3_open" "sig" "sqlite3"`. This was built once
  and reverted: it works end to end but needs a persisted IR field in
  `src/ir_schema_gen.rs`, which is `@generated` yet hand-patched, so a clean
  regen rewrites ~1300 lines. Correct, and the cost is a day of generator work,
  not a design question.
- **Manifest globs** — `[c.libs.sqlite3] symbols = ["sqlite3_*"]`. No language
  change, and it covers the common case where a library has a symbol prefix. It
  is a heuristic, and a library without a prefix defeats it.
- **Grouped declarations** — a `#c` block introduced by its library. Cleanest to
  read, largest parser change.

**Recommendation: the third string**, with the manifest glob as the interim that
needs no IR change. The third string is the only one that makes the attribution a
FACT the compiler holds rather than a pattern it guesses, and availability is a
question whose wrong answer is silent.

## Finding it, and finding it again at run time

Two different questions that currently share code by accident.

**Load time** — an ordered search, stated once: beside the package (a vendored
copy), then `~/.loft`'s registry copy, then the host's own linker search
(`LD_LIBRARY_PATH` / `DYLD_LIBRARY_PATH` / `PATH`, the system directories, and on
macOS the dyld shared cache). `extensions::load_c_library` already does the first
and last; it should not grow a second copy of the middle.

**After linking** — the built binary has to find the library again, and the three
targets do not share a mechanism: ELF `-rpath $ORIGIN`, Mach-O
`-rpath @loader_path` with `@rpath/` install names, and Windows nothing at all —
the DLL is staged beside the `.exe`. The general rule is *"make it findable from
the artifact's own location"*; the three implementations are a per-target detail
of the same resolver, which is where the `-Wl,-rpath`-on-MSVC noise came from
(one arm applied to all three).

**Shims are ordinary libraries.** A built shim gets an identity like any other
(`{pkg}_shim@0`), so its `.so` / `.dylib` / `.dll`, its Windows import library and
its macOS install name all fall out of the same table rather than out of three
bespoke helpers.

## wasm and the browser

The identity resolves to **no candidate** on `wasm32` targets, and that is a
defined answer rather than a gap. Today `--native-wasm` on a `#c` package emits
generated Rust calling `loft::c_call::resolve_native`, which is behind the
`native-extensions` feature, so the author reads `E0433: cannot find c_call in
loft` once per bound symbol plus a note about loft's own `src/lib.rs`. The
resolver returning "unsupported on this target" gives one message naming the
package and the library, **before** codegen.

## Build ladder

Each step lands green alone, and the order puts the falsifiable test first.

| step | what it proves | how |
|---|---|---|
| **N1** | the property is checkable and FAILS today | the conformance test above over every `[c]` declaration × three targets; the Windows column must go red before anything is fixed |
| **N2** | the resolver exists and changes nothing | `CLib` + `candidates(target)`; every existing declaration produces byte-identical spellings to today's, except the Windows link stem the test just caught |
| **N3** | one home | sites 2–6 consult the resolver; each removal is a deletion, and the conformance test is what says the deletion was safe |
| **N4** | the identity spelling parses | `libs = ["sqlite3@0"]` alongside the filename form, with the old form DEFINED as the identity it maps to |
| **N5** | overrides | per-target stem override, with a test that it replaces the stem and not the search |
| **N6** | wasm refuses by name | the resolver's empty answer becomes one message before codegen (this is @PLN23 P6) |
| **N7** | symbol attribution | the manifest glob, then the third string once `ir_schema_gen` is regenerable |

N1 before N2 is not ceremony: a resolver written first would be measured against
the spellings it produced, and the Windows bug is invisible that way.

## What this does not address

- **Which ABI a library actually has.** The identity says `mariadb@3`; nothing
  checks the loaded library really exports that generation. `library_available`'s
  symbol check is the current approximation, and version skew is real
  (`maria/src/stmt.c` hand-declares `MYSQL_BIND` against a pinned soversion).
- **Architecture.** The trampolines' register/stack split is @PLN24 arc B and is
  orthogonal to naming.
- **Vendoring.** Shipping a copy of a C library inside a package is @PLN21's
  question; this design only says where the search would look for it.

## See also

- [README.md](README.md) — @PLN24, and the parity table this fills the naming row of.
- [@PLN23 PLATFORMS.md](../23-db-clients/PLATFORMS.md) — the four platform cells,
  and where the measurements above came from.
- [PACKAGES.md § Direct C binding](../../PACKAGES.md) — the reference home this lands in.
