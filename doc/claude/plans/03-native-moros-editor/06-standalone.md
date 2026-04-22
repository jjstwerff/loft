<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 6 — Standalone compiled application

**Status:** ✅ done 2026-04-22.  `make editor-dist` produces a
relocatable `dist/moros-editor/` bundle (binary + cdylib + font,
`$ORIGIN` rpath).  Verified by copying the dist dir to
`/tmp/ne_relocated` and running from there — the binary loaded
the sibling cdylib and entered its main loop, exiting at
`gl_create_window` only because this dev box has no display.

## Scope

Make the native Moros editor shippable as a self-contained binary a
user can run without having `loft` installed.  Today `loft --native`
produces a runnable binary but it's cached under
`<script_dir>/.loft/cache/<stem>-<hash>` and the invocation expects
`loft` in `$PATH` to regenerate when the source changes.  For
distribution, the artifact has to be self-identifying, self-contained,
and produced by a single make target.

## What `loft --native` already gives us

- A cached ELF / Mach-O / PE binary at
  `<script_dir>/.loft/cache/<stem>-<sourcehash>`.  The binary links:
  - `libloft.rlib` (static — baked in).
  - `libloft_ffi.rlib` (static — baked in).
  - `liballoc` / `libstd` / `libcore` (static by default).
  - **Dynamic**: `libloft_graphics_native.so` (cdylib exporting
    `loft_gl_*` symbols).  This MUST ship alongside.
  - **Dynamic**: any other `*-native` cdylib referenced by packages the
    script imports.
  - System libraries — `libGL`, `libX11` / `libwayland-client` on
    Linux, `OpenGL.framework` / `Cocoa.framework` on macOS, `opengl32.dll`
    / `gdi32.dll` on Windows.  These are assumed present on the target
    system.

`--native-release` additionally applies `-O` so the binary is
optimised.

## Distribution model

Ship as a directory layout (matches how games, Electron apps, and
most Linux binaries ship today):

```
moros-editor/
├── moros-editor            (executable — renamed from the .loft/cache path)
├── libloft_graphics_native.so  (next to the binary; loader resolves via rpath)
└── assets/
    └── DejaVuSans-Bold.ttf     (from lib/graphics/examples/)
```

`rpath=$ORIGIN` on the binary makes it find the adjacent `.so`
without `LD_LIBRARY_PATH`.

## Work

### 1. `make native-editor` target in the repo Makefile

```make
.PHONY: native-editor
native-editor:
	cargo build --release --lib --bin loft
	cargo build --release --manifest-path=lib/graphics/native/Cargo.toml
	./target/release/loft --native-release \
	    --path . --lib lib \
	    lib/moros_editor/examples/native_editor.loft
	mkdir -p dist/moros-editor/assets
	cp lib/moros_editor/examples/.loft/cache/native_editor-*  \
	   dist/moros-editor/moros-editor
	cp lib/graphics/native/target/release/libloft_graphics_native.so \
	   dist/moros-editor/
	cp lib/graphics/examples/DejaVuSans-Bold.ttf \
	   dist/moros-editor/assets/
	patchelf --set-rpath '$$ORIGIN' dist/moros-editor/moros-editor
	@echo "distributable: dist/moros-editor/"
```

The `patchelf` invocation sets rpath so the binary finds the
adjacent `.so` at runtime without environment tweaking.  On macOS
the equivalent is `install_name_tool -add_rpath @executable_path`;
on Windows the `.dll` just needs to be next to the `.exe`.

### 2. Asset loading discipline in the driver

The driver must not hardcode `lib/graphics/examples/DejaVuSans-Bold.ttf`
(that path only exists in the source tree).  Instead:

```loft
// Look for assets next to the binary, fall back to source-tree path
// for `loft --native ...` development runs.
font_path = "assets/DejaVuSans-Bold.ttf";
if !file(font_path).exists() {
    font_path = "lib/graphics/examples/DejaVuSans-Bold.ttf";
}
```

This keeps `loft --native` development working AND produces a
binary that finds its font in the distribution layout.

### 3. Optional: `--native-bundle` flag on loft

A convenience: `loft --native-bundle dist/my-app/ script.loft`
would perform the copy+patchelf dance in one invocation.
**Defer** — the Makefile target is enough for the first round.
Users running the editor directly from source don't need this;
only distribution does.

### 4. CI job (optional, later): build dist artifact per-platform

GitHub Actions matrix builds for linux-x64, macos-arm64,
windows-x64.  Each uploads `moros-editor-<platform>.tar.gz`.
**Defer** — not part of MVP.

## Test plan

1. `make native-editor` succeeds on the dev machine.  Produces
   `dist/moros-editor/` with the three files.
2. `./dist/moros-editor/moros-editor` opens a window from the dist
   directory (not the source tree).  Exits cleanly on Esc.
3. Move `dist/moros-editor/` to a fresh directory (e.g. `/tmp/fresh/`).
   Run the binary from there.  Must still work — proves the rpath is
   correct and there are no hardcoded absolute paths.
4. `ldd dist/moros-editor/moros-editor` on Linux shows only:
   - `libloft_graphics_native.so => ./libloft_graphics_native.so`
   - System libraries.

## Acceptance

- [ ] `make native-editor` produces `dist/moros-editor/` with binary
      + font + graphics-native cdylib.
- [ ] Binary runs from the dist dir (not from source tree).
- [ ] Binary runs after the dist dir is relocated.
- [ ] `ldd` shows rpath-resolved cdylib, no stray dev-path deps.
- [ ] Font + any other assets load from `./assets/` relative to the
      binary.

## Non-goals

- **`.deb` / `.rpm` / `.msi` installers.**  Tarball is enough.
- **Code signing.**  Out of scope.
- **Auto-update.**  Separate concern.
- **Single-file binary** (e.g. `warp-packer` or similar).  The
  directory-with-libs layout is standard and well-understood; a
  single-file packer adds complexity for marginal polish.

## Rollback

Makefile addition is self-contained.  Asset-loading fallback in the
driver is one `if` block.  Both can revert without touching
Phase 0–5 code.
