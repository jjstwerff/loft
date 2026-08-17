<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN142 — Sudo-free user-local install of loft

> Tracker: [loft-lang/plans#142](https://github.com/loft-lang/plans/issues/142)
> (`subject:loft` + `status:next`).

## Status

Phases 1–3 **implemented + validated live** on branch `mac-install-dylib-fix`
(commits `e37266a1`, `75a7b67d`), pending merge to `main`. `make install-user`
installs to `~/.local` with no sudo, passes the stdlib + cdylib smokes, and reports
`PATH OK`.

## Goal

Let a developer install loft into a **user-writable prefix** (`~/.local` by
default) with **no `sudo`**, so loft can be reinstalled routinely — after each PR
merge — without root. Today the only install path is system-wide
(`/usr/local/{bin,share}`), which needs root and breaks too often to run frequently.

**Why it's cheap:** the runtime already resolves its stdlib + rlibs **relative to
the binary** — when the binary sits in `<prefix>/bin/loft`, it searches
`<prefix>/share/loft` and `<prefix>/share/loft/deps` (`src/cache.rs:416`,
`src/native_lib.rs:805`). Proven: the current post-install smoke runs the installed
stdlib from `/usr/local/share/loft`. So a `~/.local` install needs **no code
change** — only Makefile parameterisation. The one hard requirement is that the
install's *content* (which dep artifacts get copied) is correct, which Phase 1 fixes.

## Motivating failure (2026-08-17)

`make install` failed at its binary↔rlib smoke with:

```
error[E0463]: can't find crate for `displaydoc` which `loft` depends on
```

**Root cause:** `displaydoc` is a **proc-macro** crate — on macOS it builds to
`libdisplaydoc-*.dylib`, with no `.rlib`. The install copies `deps/*.rlib`
(Makefile:235) and `deps/*.so` (Makefile:237) but **never `*.dylib`**. The installed
`share/loft/deps/` ends up with 107 `.rlib`, 0 `.dylib`, 0 `.so` — so any consumer
library's cdylib (`extern crate loft;`) can't resolve loft's proc-macro dep and
fails E0463. macOS-specific: on Linux proc-macros are `.so`, which IS copied. This
is a concrete instance of "install is broken too often" and blocks even the
system-wide install today.

## Phases

Cut per the two-bounds rule; each can go red on its own.

### Phase 1 — Copy proc-macro `.dylib` deps (XS) — unblocks install now

Copy `deps/*.dylib` alongside `*.rlib`/`*.so` in `install-artifacts`/`install`, so
the installed `share/loft/deps/` carries every artifact a cdylib link needs.
- **Validation (can go red — it just did):** the existing post-install cdylib smoke
  (Makefile ~279) goes red→green. Confirm on macOS (`.dylib`) *and* that Linux
  (`.so`) is unaffected.
- Note: `uninstall`'s `rm -rf share/loft` already removes them; the `rm -f
  deps/*.so`/`*.rlib` refresh lines need a `*.dylib` sibling so a reinstall doesn't
  leave a stale proc-macro dylib.

### Phase 2 — `PREFIX` parameterisation + conditional sudo (S) — no code change

Make `install`/`uninstall` honour `PREFIX ?= /usr/local`. Replace the hardcoded
`/usr/local/{bin,share}` with `$(PREFIX)/...`. Gate `sudo` on writability: when
`$(PREFIX)/bin` and `$(PREFIX)/share` are user-writable (or creatable), run the
copies **without** `sudo`; keep the escalation only for a non-writable prefix.
- **Validation (can go red):** `make install PREFIX=$HOME/.local` completes with
  **no sudo prompt**, and `~/.local/bin/loft` passes the SAME stdlib + cdylib smoke
  (proves prefix-relative resolution end-to-end). A stray `sudo` would prompt; a
  non-prefix-relative resolver would fail the smoke.
- Keep the existing `sudo true ||` preflight only on the system path — a user-prefix
  install must never demand root.

### Phase 3 — `make install-user` + per-OS PATH verification + the reinstall loop (S)

- `install-user` target = `install` with `PREFIX := $(HOME)/.local`.
- **PATH verification (per-OS) — the install must confirm the freshly installed
  loft is the one the shell resolves.** A binary in `$(PREFIX)/bin` that isn't on
  `PATH` — or is *shadowed* by an older loft earlier on `PATH` (e.g. a system
  `/usr/local/bin/loft` in front of a new `~/.local/bin/loft`, or vice versa) — means
  `loft` on the command line silently isn't the one just installed. So after the
  copy, verify:
  1. `$(PREFIX)/bin` is a `PATH` entry, **and**
  2. `command -v loft` resolves to exactly `$(PREFIX)/bin/loft` (catches shadowing,
     not just absence).
  If either fails, print the **OS-appropriate** remedy rather than a generic
  `export` (don't edit the user's rc):
  - **macOS** (zsh default): the line for `~/.zprofile`
    (`export PATH="$HOME/.local/bin:$PATH"`), and note `/etc/paths.d/` as the
    system-wide alternative; flag when an earlier `/usr/local/bin/loft` shadows it.
  - **Linux** (bash/other): the line for `~/.profile` / `~/.bashrc`; note that
    `~/.local/bin` is on `PATH` by default under many distros' `~/.profile`
    (systemd `user-dirs`), so the check may already pass.
  - Resolve the shell from `$SHELL`/OS rather than assuming, since the rc file
    differs (`~/.zprofile` vs `~/.bash_profile` vs `~/.profile`).
- **Optional fast path:** a lighter variant that skips the wasm/html-mt artifacts
  (the slow part of `install-artifacts`) for a native-only "after each PR merge"
  reinstall — the browser runtimes aren't needed to run loft locally. Decide in this
  phase whether to fold it into `install-user` or a separate `install-user-fast`.
- **Validation (can go red):** run the target with `$(PREFIX)/bin` absent from
  `PATH` → it warns with the correct per-OS line; with it present but a system loft
  ahead of it → it reports the shadowing; with it correctly first → `command -v loft`
  equals `$(PREFIX)/bin/loft` and the check is silent. All three are real,
  reproducible states.
- Doc: a short section (RELEASE.md or a new INSTALL note) on the no-sudo reinstall
  loop, including the PATH one-liner per OS.

## Open questions

- **Writability detection** — test-and-create `$(PREFIX)` vs check `-w`. A
  first-time `~/.local/share` may not exist; `install -d` as the user creates it,
  so detection should treat "creatable by me" as writable.
- **Symlink vs copy for the binary** — `install-user` could symlink
  `target/release/loft` into `~/.local/bin` (like the `debug` target does) for an
  even faster reinstall, but then `share/loft` must still be refreshed as a copy;
  a copy keeps the binary↔stdlib pair atomic, which is what the smoke guards. Lean
  copy.
- **`PREFIX` for wasm/html-mt** — those artifacts are large; the fast path above
  likely makes them opt-in for a user install.

## See also

- `src/cache.rs:410` `loft_rlib_path` / `src/native_lib.rs:792` `rlib_search_dirs` —
  the prefix-relative resolvers that make this a Makefile-only change.
- Makefile `install` / `install-artifacts` / `uninstall` (221 / 214 / 293).
- loft#693 — the binary↔rlib smoke this plan keeps green.
