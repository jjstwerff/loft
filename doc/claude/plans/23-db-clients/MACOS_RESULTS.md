<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# macOS results — the answers MACOS_HANDOFF.md asked for

Run on Apple silicon (`/opt/homebrew`), macOS (Darwin 25.6). Every number below is
measured on this box; nothing is inferred. The verdict against each "what would
change our minds" hook is stated at the end.

**Headline:** P2 is **confirmed on macOS** — sqlite now runs. All five tasks
answered; the only two full-suite failures are an unrelated pre-existing flake.

## Task 1 — the fixed probe runs sqlite on macOS: **CONFIRMED**

```
@PLN23 backends exercised: ["sqlite"]
test one_sql_interface_drives_four_different_c_libraries ... ok  (8.26s)
```

- `sqlite` present → **P2 confirmed on macOS**, and P3-for-sqlite with it.
- The three assertion lines (`sqlite <value/NULL/empty>`, `sqlite bound …`,
  `sqlite tx …`) are hard-coded constants in `tests/native.rs` — the **same**
  strings Linux matches. sqlite passed all three, and the test's own
  `assert_eq!(run("--native"), run("--interpret"))` held. So the uniform API is
  uniform on macOS too (see the "change our minds" verdict below).
- `postgres` / `maria` / `duckdb` are absent from the list — not a failure, a
  correct skip. They are keg-only / not shipped as a dylib; see Task 3.
- `has()` is `loft::platform::host_library_loadable`, so this exercised the fixed
  dlopen probe, not the old Linux-shaped `Path::exists`.

## Task 2 — the dyld shared cache: **CONFIRMED — the `dlopen` design is REQUIRED**

```
ls -l /usr/lib/libsqlite3.dylib      → No such file or directory   (stat exit: 1)
dlopen('libsqlite3.dylib')           → OK
dlopen('libsqlite3.0.dylib')         → OK
```

`stat` fails, `dlopen` succeeds — for **both** spellings `host_lib_variants`
produces. So a `Path::exists` probe would report "not installed" for a library
that works, exactly as the comment in `host_library_loadable` claims. The comment
stands as written; do not cut it back.

## Task 3 — Homebrew kegs are keg-only: **CONFIRMED — P5 needs a search path, not `brew install`**

```
brew --prefix                        → /opt/homebrew
libpq / mariadb-connector-c / duckdb → all installed under /opt/homebrew/opt/<name>
/opt/homebrew/opt/libpq/lib/libpq.5.dylib                 present
/opt/homebrew/opt/mariadb-connector-c/lib/libmariadb.3.dylib  present

dlopen('libpq.5.dylib')       → FAIL ('… not in dyld cache')
dlopen('libmariadb.3.dylib')  → FAIL ('… not in dyld cache')
DYLD_LIBRARY_PATH=$(brew --prefix libpq)/lib  dlopen('libpq.5.dylib')  → OK
```

The libraries are installed but **not on the default `dlopen` search path**, so
`host_library_loadable("libpq.so.5")` answers **false** on a Mac that has libpq
installed. P5 on macOS is therefore a **search-path** answer (a `DYLD_LIBRARY_PATH`,
or loft learning Homebrew's `/opt/homebrew/opt/<x>/lib` layout), not a
`brew install`. **PLATFORMS.md's keg-only warning STANDS — keep it.**

### Correction owed to PLATFORMS.md: `brew install duckdb` ships no dylib

```
find /opt/homebrew -name 'libduckdb*'   → (nothing)
/opt/homebrew/opt/duckdb/lib/            → does not exist
```

`brew install duckdb` installs only the **CLI binary** — there is no
`libduckdb.dylib` anywhere under Homebrew. So the "Getting the libraries onto a
machine" table row **macOS / duckdb = `brew install duckdb`** is wrong for this
plan: the shared library needs the **release tarball**, same as the Debian row
already says. Recommend changing that cell to `release tarball` (or
`release tarball; brew ships CLI only`).

Net for the probe on macOS: it finds **only sqlite** (base system, in the dyld
cache). libpq/libmariadb are keg-only; duckdb has no dylib. All three are
`[c] optional-libs`, so each is a correct reported skip — which is what Task 1
shows.

## Task 4 — the shim's install name: **CONFIRMED correct**

`otool -D` on every shim built by the fixtures:

```
@rpath/libsqlite_shim_9d0258cd31a654bc.dylib
@rpath/libpostgres_shim_47ee541dd3f6d6d5.dylib
@rpath/libduckdb_shim_08138727af25f4c6.dylib
@rpath/libmaria_shim_472e8fbb0cb8d9ba.dylib
```

Every one is the **final** `@rpath/lib<x>_shim_<key>.dylib` — no `.tmp`, no build
directory. `otool -L` on the sqlite shim: it names itself `@rpath/…_shim_…` and
depends only on `/usr/lib/libSystem.B.dylib`. dyld accepts the result; the flag
does what its unit test asserts. `c_binding` suite: 4 passed.

### Side observation (not the asked artifact, latent)

The **other** artifact in each `native-auto/` dir — the top-level program cdylib
`libloft_auto_<mode>_<hash>.dylib` — carries an install name of

```
/Users/…/native-auto/loft_auto_<mode>_<hash>.building
```

i.e. the `.building` temp stem **with the absolute build directory baked in**.
That is the exact `.tmp`-in-install-name shape the handoff flags — but on the
program cdylib, which is loaded **by path** (dlopen-by-path ignores the install
name), so it breaks nothing here and the suite is green. Worth a glance only if
these `loft_auto_*` cdylibs are ever consumed via `@rpath` rather than by path.

## Task 5 — the full suite: **3718 / 3720 pass; the 2 failures are an unrelated flake**

```
Summary [414.816s] 3720 tests run: 3718 passed, 2 failed, 24 skipped
FAIL loft::multiplayer_v5 v5_t2_session_blob_grouping
FAIL loft::multiplayer_v5 v5_t4_catch_up_after_reconnect
```

Both failures are in `multiplayer_v5`, **not @PLN23**, and both are the same root
cause — a registry auto-install that could not unpack:

```
[registry] auto-install failed for server: extract …/web-0.3.4.tar.gz:
    failed to unpack `…/web-0.3.4/native/src/lib.rs`
error: Library 'server' not found — searched lib/, lib_dirs, and sibling packages
```

`~/.loft/registry/web-0.3.4/` is already fully populated from an earlier run, so
this is a **parallel-extraction race**: two v5 tests auto-install the same shared
`web-0.3.4` package into the same directory concurrently and collide. Decisive
check — re-run the two serially:

```
cargo nextest run --release --test multiplayer_v5 -j1 \
    v5_t2_session_blob_grouping v5_t4_catch_up_after_reconnect
→ 2 passed  (2.4s)
```

Green on serial re-run. This is a pre-existing test-isolation issue in the
registry auto-installer (no lock around extract-into-shared-cache), environment-
triggered under nextest parallelism. It is **not** macOS-specific and **not** a
regression from P1/P2. Aside from it, the macOS suite is green.

## What would change our minds — the verdicts

- **Task 2 stat works on macOS?** No — `stat` fails, `dlopen` succeeds. The
  shared-cache argument holds; the `dlopen` probe is required, not a preference.
  The `host_library_loadable` comment needs no cutting back.
- **Task 3 kegs on the default search path?** No — keg-only confirmed (bare
  `dlopen` fails, `DYLD_LIBRARY_PATH` fixes it). P5's keg-only warning stays.
  Plus: `brew install duckdb` ships no dylib at all — PLATFORMS.md correction owed.
- **Task 1 sqlite running but assertions differing from Linux?** No — the
  expected strings are identical constants and macOS matched them, with
  `--interpret` == `--native`. The uniform API is uniform on macOS.

## Recommended PLATFORMS.md edits (left for the branch owner — not applied here)

Per the handoff ground rule "do not push to `tuxedo-windows-c-shim`", these are
written down rather than applied to the concurrently-edited file:

1. **macOS row** of the "Where it stands — measured" table: `was green and empty`
   → **green** (sqlite measured, P2/P3-sqlite confirmed).
2. **"Getting the libraries" table**, macOS / duckdb cell: `brew install duckdb`
   → `release tarball` (brew ships the CLI only; no `libduckdb.dylib`).
3. Optionally note under macOS that libpq + mariadb-connector-c are **installed
   and keg-only** on this box, so P5 is confirmed to be a search-path question.
