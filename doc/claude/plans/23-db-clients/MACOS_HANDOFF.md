<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Handoff — what a macOS agent should run, and what would change our minds

You are on hardware nobody else here has. Everything in this file is a question
that **cannot be answered from Linux**, ordered by what it unblocks. Read
[PLATFORMS.md](PLATFORMS.md) first; this is the task list that hangs off it.

## The one-line context

@PLN23 is a SQL client over four C libraries bound through `#c` (@PLN24). It is
proven on Linux. macOS has been **green while opening no database** — the
availability probe was written in Linux's library spelling, so every conditional
cell skipped in silence. That is now fixed (P2), and this file exists because
nobody has yet watched the fixed version run on a Mac.

## Ground rules

- **Do not push to `tuxedo-windows-c-shim`.** It is being worked on concurrently.
  Use your own branch, or just run and report. Report findings back into this
  file (or a sibling), not into a consumer tree.
- Answers belong in the repo, not only in a reply — this file is the shared
  channel.
- If a command below fails for a boring reason (no Homebrew, no Xcode CLT), say
  so plainly. "Could not run" is a result; a guess is not.

## Task 1 — does the fixed probe actually run sqlite on macOS?

This is the whole point of P2 and the only thing that closes the macOS row.

```bash
cargo nextest run --release --test native one_sql_interface --no-capture
```

**What to look for.** The test prints one line naming the backends that ran:

```
@PLN23 backends exercised: ["sqlite", ...]
```

- `sqlite` present → **P2 is confirmed on macOS.** Report the full line.
- `sqlite` absent → the probe still cannot see the system sqlite. That is the
  interesting failure; go to Task 2.

The probe is `platform::host_library_loadable` (`src/platform.rs`). It takes the
declared Linux soname `libsqlite3.so.0`, asks `host_lib_variants` for this host's
spellings (`libsqlite3.0.dylib`, `libsqlite3.dylib`), and **`dlopen`s** them.

## Task 2 — the dyld shared cache, which is why it `dlopen`s rather than stats

The reason the probe opens the library instead of looking for the file is a claim
I could not verify from here, and it decides whether the implementation is right:

> On macOS 11+, system libraries live in the **dyld shared cache** and are not
> present on disk, so `/usr/lib/libsqlite3.dylib` does not exist as a file while
> `dlopen("libsqlite3.dylib")` succeeds.

```bash
ls -l /usr/lib/libsqlite3.dylib ; echo "stat exit: $?"
python3 -c "import ctypes; ctypes.CDLL('libsqlite3.dylib'); print('dlopen: OK')"
python3 -c "import ctypes; ctypes.CDLL('libsqlite3.0.dylib'); print('dlopen 0: OK')"
```

**Why it matters.** If `stat` fails and `dlopen` succeeds, the `dlopen` design is
*required*, not merely tidier — and that belongs in `PLATFORMS.md` as a measured
fact. If BOTH fail, `host_lib_variants` is producing the wrong spelling for macOS
and that is a real bug in `lib_variants` worth its own fix.

## Task 3 — where Homebrew actually puts libpq and the MariaDB connector

`PLATFORMS.md` currently states these from general knowledge, **flagged as a
guess**. Please replace them with measurements.

```bash
brew --prefix
brew install libpq mariadb-connector-c duckdb   # or report what is already there
brew --prefix libpq ; brew --prefix mariadb-connector-c
ls -l "$(brew --prefix libpq)/lib/"*.dylib 2>/dev/null | head
python3 -c "import ctypes; ctypes.CDLL('libpq.5.dylib'); print('libpq bare: OK')"
```

**The question:** are they **keg-only** — reachable only under
`$(brew --prefix libpq)/lib` and NOT on the default `dlopen` search path? If so,
`host_library_loadable("libpq.so.5")` answers false on a Mac that has libpq
installed, and P5 needs a search-path answer (a `DYLD_LIBRARY_PATH`, or loft
learning Homebrew's layout) rather than a `brew install`.

Then re-run Task 1 and report whether `postgres` / `maria` joined the list.

## Task 4 — the shim's install name, on a Mac that can check it

macOS bakes the `-o` path into a dylib as its install name, and loft builds shims
to `<stem>.<pid>.tmp` before renaming. `platform::shim_name_args` passes
`-Wl,-install_name,@rpath/<final name>` to fix that. It has a unit test, but the
test asserts the FLAG, not that dyld accepts the result.

```bash
cargo nextest run --release --test native c_binding
otool -D tests/fixtures/sqldb/sqlite/native-auto/*.dylib
otool -L tests/fixtures/sqldb/sqlite/native-auto/*.dylib
```

**Expected:** the install name is `@rpath/libsqlite_shim_<key>.dylib` — the FINAL
name, with no `.tmp` and no build directory in it. A `.tmp` there is the bug that
already shipped once.

## Task 5 — the full suite, because nobody has read a macOS failure list

```bash
./scripts/find_problems.sh --bg    # then --peek / --wait
```

macOS runs the full suite on push-to-main and daily, so this is not new signal —
but a local run lets you actually *investigate* a failure instead of reading a
log. If it is all green, say so; that is worth recording too.

## What we would change our minds about

Stated up front so the answers are not fitted to a conclusion:

- **If Task 2 shows `stat` works on macOS**, the shared-cache argument is wrong
  for this library and the `dlopen` probe is merely a preference. Say so — the
  comment in `host_library_loadable` claims more than that and would need cutting
  back.
- **If Task 3 shows the kegs ARE on the default search path**, then P5 on macOS is
  just provisioning and `PLATFORMS.md`'s warning about keg-only paths should go.
- **If Task 1 shows sqlite running but the three assertion lines differing from
  Linux's**, that is far more interesting than a skip: it means the uniform API is
  not uniform across platforms, which is a claim @PLN23 makes everywhere. Capture
  both outputs verbatim.

## Not your problem

- **Windows.** `LNK1181`, the import library, `--out-implib`. Needs Windows.
- **`c_library_available` answering false for an installed library.** Reproduces
  on Linux; being handled here. Do not build anything on that function — the
  probe above deliberately avoids it.
- **wasm.** A declared gap; a browser cannot open a socket to a database.
