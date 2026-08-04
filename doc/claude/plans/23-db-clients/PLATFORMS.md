<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 23 — building the db client on every platform

The client is four C libraries reached through `#c` (@PLN24), so "does it work"
is a different question on every host: the library has a different **name**, the
shim needs a different **link**, and on one target the capability does not exist
at all.

## The bar

@PLN24 sets it, and it is deliberately not *works everywhere*:

> Parity is **"a defined answer in every column"**, not *"works everywhere
> automatically"*.

`#native` does not get the browser for free either. So this document is finished
when the table below has no blank cell — each one a green, or a refusal that
names itself.

## Where it stands — measured

| platform | the shim links | the C library resolves | the four backends | state |
|---|---|---|---|---|
| Linux x86-64 | yes | yes | proven, both loft backends | **green** |
| macOS | unproven | untested — the probe never asks | **not exercised** | **unknown** |
| Windows | **no** — `LNK1181` | untested — the probe never asks | **not exercised** | **red** |
| wasm / browser | n/a | n/a | impossible by capability | **owed a clear refusal** |

Only the Linux row is a measurement of the thing the plan claims. The other three
are the subject of this document, and two of them are *worse than red* — they are
cells that pass without testing anything.

## Two ways a green here means nothing

Both have already happened, and neither is visible from a check name.

**1. The check name is a placeholder.** `Test (windows-latest)` and
`Test (macos-latest)` in the PR list run on an `ubuntu-latest` runner and only
echo a notice. The real matrix job renders under the *same display name* when it
runs, so a placeholder and a real run are indistinguishable in `gh pr checks` —
read the runner OS or the log, never the name. Per-PR macOS coverage is real but
narrow (Miri-macOS, ASan-macOS) and **excludes `tests/native.rs`**, which is
where every fixture in this plan lives. For a real answer, dispatch one:

```bash
gh workflow run ci.yml --ref <branch> -f os=macos-latest
gh workflow run ci.yml --ref <branch> -f os=windows-latest
```

**2. The availability probe is Linux-shaped.** `one_sql_interface_drives_four_different_c_libraries`
gates each backend on

```rust
["/lib/x86_64-linux-gnu/", "/usr/lib/", "/usr/lib64/"]   // Linux dirs
    .iter().any(|d| Path::new(&format!("{d}{n}")).exists())   // n = "libsqlite3.so.0"
```

Both the directories and the spelling are Linux's. On macOS and Windows this is
**always false**, so the sqlite, postgres and maria cells are skipped in silence
— including sqlite, the cell whose entire job is to be the one that always runs.
What remains is the unconditional duckdb cell, and that is exactly the one the
Windows run died in (`--native/duckdb`).

So on a non-Linux host the test today proves *the duckdb shim compiles and links*
and nothing else about this plan. Fixing the link without fixing the probe buys a
green that still tests no database.

## Windows

### What landed

`7066771c` — `-l` names a library, so the stem strips `.dll` beside `.so` /
`.dylib`, and `-Wl,-rpath` is no longer passed on a host that has no RPATH. That
turned `sqlite_shim_<hash>.dll.lib` into a request for the right name.

### What the run then reported

```
LINK : warning LNK4044: unrecognized option '/Wl,--allow-multiple-definition'; ignored
LINK : fatal error LNK1181: cannot open input file 'sqlite_shim_5aed31d6bafbf9f8.lib'
```

The name is now right and **the file does not exist** — the follow-up the landed
commit predicted in its own message. Two changes remain:

**W1 — the shim must emit an import library.** `c_shim.rs` builds every shim with
`-O2 -fPIC -shared`, and a MinGW `cc` emits no import library unless asked. MSVC
`link.exe` cannot link a bare `.dll`; it needs the `.lib`. The hook already
exists and already has a per-OS arm — `platform::shim_name_args`, which is where
macOS's `-Wl,-install_name` lives — so this is a Windows arm on a function that
was built for exactly this, adding `-Wl,--out-implib,<stem>.lib`.

Worth naming before it is attempted: a MinGW-produced COFF import library is
normally consumable by MSVC for plain cdecl x64 C symbols, but that is the
assumption this step is testing. If it does not hold, the fallbacks in order of
preference are to build the shim with MSVC directly (`cl /LD`), or to link the
shim's objects statically into the binary and drop the DLL entirely — the shims
are three-line trampolines that reference no database symbol, so static linking
costs little and removes the import-library question outright.

**W2 — stop sending MSVC a GNU linker flag.** `-Wl,--allow-multiple-definition`
is emitted at `main.rs` and `test_runner.rs`, each guarded `#[cfg(not(target_os =
"macos"))]` because ld64 rejects it. MSVC rejects it too, and says so three times
directly above the real error. The guard needs Windows for the same reason macOS
has it. This fixes no link by itself; it stops the log lying about where the
failure is.

### After that, the databases themselves

A Windows runner ships none of these libraries. All four are declared
`[c] optional-libs`, so an absent one is a reported SKIP rather than a failure —
which means **W1 + W2 turn the Windows cell green while still testing no
database**. That is the second failure mode above, so the Windows work is not
done at W2; it is done when the probe asks the host's own question (P2 below) and
sqlite answers it.

## macOS

Nothing is known to be broken, and nothing is known to work — `tests/native.rs`
has never run there for this fixture. The pieces that must line up:

- **The library name.** `platform::lib_variants` translates the declared Linux
  spelling at use time. macOS system sqlite is `/usr/lib/libsqlite3.dylib`, which
  the translation already produces.
- **The shim's install name.** `shim_name_args` already passes
  `-Wl,-install_name,@rpath/<final name>` on macOS, and already accounts for the
  build-to-`.tmp`-then-rename publish that once shipped a `.tmp` install name.

| declared | macOS is asked for | Windows is asked for |
|---|---|---|
| `libsqlite3.so.0` | `libsqlite3.0.dylib`, `libsqlite3.dylib` | `sqlite3.dll`, `libsqlite3.dll` |
| `libpq.so.5` | `libpq.5.dylib`, `libpq.dylib` | `pq.dll`, `libpq.dll` |
| `libmariadb.so.3` | `libmariadb.3.dylib`, `libmariadb.dylib` | `mariadb.dll`, `libmariadb.dll` |
| `libduckdb.so` | `libduckdb.dylib` | `duckdb.dll`, `libduckdb.dll` |

The declared spelling is always tried first and stays authoritative; these are
what a host is asked for when its own convention differs.

## wasm and the browser — a defined NO

**This target is a known gap and stays one.** A browser cannot open a TCP socket
to a database server, so no amount of build work makes libpq or libmariadb
reachable; @PLN24 arc E names a database client as its example of a capability
that does not exist in wasm. The out-of-process route (@PLN119) is a real answer
and is not this plan's.

What is owed is therefore not support — it is a **clear error**. Today,
`--native-wasm` on a `#c` package emits generated Rust that cannot compile, and
the author reads rustc's account of loft's internals:

```
error[E0433]: cannot find `c_call` in `loft`
  --> /…/loft_wasm_3225277/prog.rs:36:32
   |
36 |   match P.get_or_init(|| loft::c_call::resolve_native("sqlite3_open", …))
   |                                ^^^^^^ could not find `c_call` in `loft`
note: found an item that was configured out
  --> src/lib.rs:145:9  |  pub mod c_call;
   ::: src/c_call.rs:34:8  |  #![cfg(feature = "native-extensions")]
```

— once per bound symbol. The cause is plain: `#c` codegen emits
`loft::c_call::resolve_native(…)` unconditionally, and `c_call` is gated behind
`native-extensions`, which a wasm target does not enable. The requirement is to
refuse where the target is chosen, naming the package and the library, instead of
emitting a call to a module that is not there. One message, before codegen —
which is also what the loft-ship cross-target gate wants, since it prefers a
declared-unsupported column to a claimed-but-broken one.

## The ladder

Each step is verifiable on its own, and the order is chosen so that no step can
produce a green that means nothing.

| step | what it proves | how it is proved |
|---|---|---|
| **P1** | the Windows shim links | W1 + W2; the duckdb cell builds and reports its SKIP instead of dying in `link.exe` |
| **P2** | the probe asks the HOST's question | `has()` takes the host spelling from `lib_variants` and searches the host's directories; sqlite stops being skipped on macOS and Windows |
| **P3** | macOS runs what Linux runs | `gh workflow run ci.yml -f os=macos-latest`; the `sqlite` / `sqlite bound` / `sqlite tx` lines byte-identical to Linux's, on both loft backends. **Count the tests in the log** — a filtered dispatch can pass vacuously |
| **P4** | Windows runs what Linux runs | the same three lines, from a Windows runner with sqlite present |
| **P5** | the servers, or a declared skip | postgres + maria on macOS/Windows are a CI-provisioning question, not a language one; a skip is fine, an untested green is not |
| **P6** | wasm refuses clearly | one named error before codegen, naming package and library — asserted on, so it cannot regress into a rustc dump |

P1 and P6 are independent of everything else and can land in either order. **P2
is the one that decides whether P3 and P4 are worth running**, so it comes before
both.

## What this does not cover

- **Which loft backend.** Every cell above must hold on `--interpret` and
  `--native` alike; the existing test already asserts the two agree, and that
  assertion carries to each new platform unchanged.
- **Architecture.** aarch64 Linux and Apple silicon differ from x86-64 in the
  register/stack split the `#c` trampolines are built on (@PLN24 arc B). Proven
  on neither; not a naming problem, so not this document's.
- **Shipping a database library.** Everything here assumes the C library is
  already on the machine. Vendoring one, or `loft install` placing it, is
  @PLN21's question.

## See also

- [README.md](README.md) — the plan, and the S-ladder this runs beside.
- [@PLN24 arc E](../24-c-abi-binding/README.md) — what a `#c` library does on
  wasm, and the three honest answers.
- [WINDOWS.md](../../WINDOWS.md) — the host's own notes.
