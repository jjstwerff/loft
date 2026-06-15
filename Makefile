# Copyright (c) 2022-2025 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# ==== What can this Makefile do for you? ================================
#
# If you just want to try things:
#
#   make play       Launch Brick Buster natively (full OpenGL window).
#                   Checks prerequisites first; fails fast with a clear
#                   message if any native library is missing.
#
#   make game       Build the Brick Buster arcade game into one HTML file
#                   (doc/brick-buster.html). Double-click to play.
#                   Works even from a half-broken checkout.
#
#   make gallery    Build the Graphics Gallery (24 demos) for the browser
#                   and verify every asset loads. Run `make serve` after.
#
#   make serve      Start a local web server on http://localhost:8000/
#                   so you can open the Playground and Gallery.
#
#   make help       Print this overview again.
#
# If you are working on loft itself:
#
#   make all        Format source + build the native binary.
#   make test       Full test suite (fmt + clippy + tests). ~1-2 minutes.
#   make quick      Same tests without the clippy/fmt gate. Faster iteration.
#   make iter TEST=<filter> [TFILE=<binary>] [PROFILE=release]
#                   Run only tests matching <filter>.  Defaults to the
#                   dev profile — incremental rebuild ~2-3s vs ~25s
#                   for release (the project's Cargo.toml turns off
#                   debug-assertions on the loft package, so dev runs
#                   loft programs at near-release speed).  Add
#                   PROFILE=release when you specifically need
#                   release behaviour or want to share cache with
#                   `make test` / `make ci`.
#   make ci         Mirror of .github/workflows/ci.yml — fmt, clippy
#                   (deny warnings), build --all-targets, build
#                   --no-default-features, nextest run --profile ci.
#                   Runs the SAME gates the remote runner uses, in the
#                   same order, so a green `make ci` predicts a green
#                   PR.  Logs to result.txt.  Does NOT run the
#                   GL / packages suites — those live in their own
#                   targets (test-packages, test-gl-smoke, test-gl-golden)
#                   and are NOT gated by the remote.
#   make ci-full    `ci` + the development-only suites (test-packages,
#                   test-gl-smoke, test-gl-golden).  What we used to
#                   call `make ci` before the slim-down.
#   make ship       Fast local pre-push gate: fmt + both clippy variants
#                   + release tests, streamed to the terminal, chained
#                   with && so `make ship && git push` stops on first
#                   failure.
#   make clean      Nuke build artifacts.
#
# More specialised:
#
#   make wasm            Build the wasm-pack bundle that drives the gallery.
#   make wasm-html-test  WASM-runtime safety gate (tests/html_wasm.rs).
#                        Rebuilds the wasm rlib in the --html shape first
#                        so the gate is deterministic regardless of whether
#                        `make wasm` last stomped the rlib with the
#                        wasm-bindgen variant.
#   make install         System-wide install (sudo).
#   make test-gl-golden  Pixel-compare the smoke-test screenshot (Xvfb).
#   make fill            Regenerate src/fill.rs from default/*.loft annotations.
#   make profile         Build with debug symbols + run a flamegraph.
#   make pdf             Rebuild the printable reference PDF.
#
# Every target above is defined as a real rule later in this file.  Scroll
# down to any name to see exactly what it does.
# =========================================================================

# Cache clean/release rebuilds with sccache when it is installed.  Exported
# so every recipe shell inherits it; a no-op when sccache is absent (CI,
# other developers) so it never becomes a hard dependency.  Composes with
# the mold linker from .cargo/config.toml (sccache caches the compile, mold
# links).  CARGO_INCREMENTAL=0 because sccache cannot cache incremental
# units — these targets are clean/release builds, not the interactive edit
# loop, so there is no cost.
ifneq ($(shell command -v sccache 2>/dev/null),)
# Only when NOT root.  sccache binds its server per-user on 127.0.0.1:4226;
# a root build (e.g. under `sudo make install`) cannot reach the invoking
# user's server and dies with "failed to read response header".  The build
# never runs as root anyway — `install` drops it back to the user via
# AS_USER below — so disabling the wrapper for the root parent is harmless.
ifneq ($(shell id -u),0)
export RUSTC_WRAPPER := sccache
export CARGO_INCREMENTAL := 0
endif
endif

# macOS: cc-rs invocations (notably `ring`'s build script) need the SDK path
# on -isysroot.  When `xcode-select -p` points at the bare Command Line Tools,
# plain `cc` does not auto-detect it and the build fails with
# `'TargetConditionals.h' file not found`.  Probe via `xcrun` and export so
# every recipe shell inherits it.  Honours a user-set SDKROOT; the whole
# block is skipped on non-Darwin (Linux/Windows), so other platforms build
# unchanged.
ifeq ($(shell uname -s),Darwin)
ifeq ($(origin SDKROOT),undefined)
SDKROOT := $(shell xcrun --sdk macosx --show-sdk-path 2>/dev/null)
endif
export SDKROOT
endif

# `make install` writes the binary to /usr/local (needs root) but the compile
# must NOT run as root: sccache is per-user (see above) and a root build would
# leave target/ owned by root, breaking the next ordinary `make`.  When invoked
# as `sudo make install` we are root with SUDO_USER set — AS_USER drops the
# build back to that user; the file-copy steps in `install` stay root.  Empty
# for a normal `make install`, where the build already runs as the user and the
# copy steps escalate with their own `sudo`.
AS_USER :=
ifeq ($(shell id -u),0)
AS_USER := $(if $(SUDO_USER),sudo -u $(SUDO_USER) -H,)
endif

.PHONY: check-wasm-threads check-no-threading par-gates gate ci-miri all check-targets doctor install install-artifacts uninstall debug test quick profile clean clean-wasm fill ci ship run-tests clippy memory last meld generate gtest pdf bench test-native test-wasm test-html-render loft-test wasm-assets test-packages test-package-native-tests test-gl-headless test-gl-smoke test-gl-golden update-gl-golden serve wasm gallery game crystal-editor play native-editor editor-dist help rebuild-native-cdylibs view-build view-refresh view index index-install-hook libcatalogue features-fetch features-gen features-check api-compat check-contract-goldens

# Print the overview at the top of this file.  Useful when you land on a
# fresh checkout and want to know what buttons are available without
# reading a 300-line Makefile.
help:
	@sed -n '/^# ==== What can this Makefile do for you/,/^# ====/p' Makefile \
	  | sed 's/^# \{0,1\}//'

all:
	@rustfmt src/*.rs --edition 2024
	@RUSTFLAGS=-g cargo build --release

check-targets:
	@if ! command -v rustup >/dev/null 2>&1; then \
		echo "WARNING: rustup not found; can't verify cross-compile targets."; \
		echo "If the build fails with E0463, install wasm32-wasip2 and"; \
		echo "wasm32-unknown-unknown manually for your toolchain."; \
		exit 0; \
	fi
	@for target in wasm32-wasip2 wasm32-unknown-unknown; do \
		if ! rustup target list --installed | grep -q "^$$target$$"; then \
			echo "Installing missing rustup target: $$target"; \
			rustup target add "$$target" || { \
				echo "ERROR: failed to install $$target."; \
				echo "Install manually:  rustup target add $$target"; \
				exit 1; \
			}; \
		fi; \
	done

# doctor: report the status of every external tool the wasm / --html / native
# pipelines depend on, and print environment-specific install commands for
# whatever is missing, so a fresh environment can be set up to actually work.
# Diagnostic only — never installs or fails the build.  See
# doc/claude/WASM.md § Build Toolchain Dependencies for what each tool is for.
doctor:
	@bash scripts/doctor.sh

# Compile every artifact that lands in /usr/local.  Runs as the unprivileged
# user (see AS_USER) — never root — so sccache and target/ ownership stay
# correct even under `sudo make install`.
#
# The three builds, in recipe order:
#   1. wasm32-wasip2 lib.
#   2. wasm32-unknown-unknown lib — W1.1, the browser target for `--html` export.
#   3. the host lib into an ISOLATED target dir, so deps/ contains exactly one
#      copy of each crate — no binary-only duplicates (e.g. libloading) that
#      cause StableCrateId collisions during native compilation.
#
# Why build 3 carries the feature list it does:
#   - `native-extensions` is REQUIRED: the `--native` codegen unconditionally
#     emits `loft::native_call::{enter,build_store}` to marshal a heap value
#     (`vector<u8>`, struct) across the cdylib FFI (src/generation/mod.rs), and
#     that module is `#[cfg(feature = "native-extensions")]` (src/lib.rs).  Drop
#     it here and the installed `libloft.rlib` lacks `native_call`, so every
#     `--native` run of a library with a `vector`/struct-returning (or -taking)
#     `#native` fn fails to compile with `E0433: cannot find native_call in loft`.
#   - `registry` + `remote-store` are BOTH in the crate's `default` features, so
#     omitting them here made the installed native runtime diverge from a normal
#     build: `store_load_url*` (registry-gated `load_url`,
#     src/database/allocation.rs) and the sibling remote/URL store loaders
#     (remote-store-gated) then `--interpret`-accept but `--native`-REJECT with
#     `no method load_url` on the stable toolchain (reported by a consumer on
#     2026.7.1).  Both pull only `ureq` (+ archive/crypto for registry) and are
#     dead-stripped from any `--native` binary that doesn't call them, so
#     bundling them is free for programs that don't fetch stores.
#
# The closing prune is the E0514 guard — cargo's deps/ accumulates a
# `libloft_ffi-<hash>.rlib` per rustc version across builds; after a `rustup
# update` the OLD one lingers beside the freshly-built one.  The install `cp`s
# ALL of them into the shared deps/, and because `cp` flattens every file's
# mtime to install-time the mtime-based loft_ffi resolver
# (native_lib::loft_ffi_for_libloft) can then pick the STALE one — so every
# `--native` build fails `E0514: crate loft_ffi compiled by an incompatible
# version of rustc`.  Keep only the newest (the just-built, current-rustc)
# loft_ffi rlib so exactly one, matching, copy is installed.
install-artifacts: check-targets all
	@cargo build --release --target wasm32-wasip2 --lib --no-default-features --features random
	@cargo build --release --target wasm32-unknown-unknown --lib --no-default-features --features random
	@cargo build --release --lib --no-default-features --features mmap,random,threading,native-extensions,registry,remote-store --target-dir target/install-lib
	@stale=$$(ls -t target/install-lib/release/deps/libloft_ffi-*.rlib 2>/dev/null | tail -n +2); \
	 if [ -n "$$stale" ]; then echo "  pruning stale loft_ffi rlib(s): $$stale"; rm -f $$stale; fi

install:
	@sudo true || { \
		echo "ERROR: 'make install' needs root to write /usr/local/{bin,share}/loft."; \
		echo "Re-run where you can elevate (e.g. as a sudoer, or 'sudo make install')."; \
		exit 1; \
	}
	@$(AS_USER) $(MAKE) --no-print-directory install-artifacts
	@$(AS_USER) $(MAKE) --no-print-directory rebuild-native-cdylibs
	@sudo install -d /usr/local/share/loft/deps
	@sudo install -d /usr/local/share/loft/wasm32-wasip2/deps
	@sudo rm -rf /usr/local/share/loft/default
	@sudo cp -r default /usr/local/share/loft/
	@sudo install -m 644 target/install-lib/release/libloft.rlib /usr/local/share/loft/
	@sudo rm -f /usr/local/share/loft/deps/*.rlib /usr/local/share/loft/deps/*.so
	@sudo cp target/install-lib/release/deps/*.rlib /usr/local/share/loft/deps/
	@if ls target/install-lib/release/deps/*.so >/dev/null 2>&1; then \
		sudo cp target/install-lib/release/deps/*.so /usr/local/share/loft/deps/ || { \
			echo "ERROR: failed to install dependency .so files (rights?)."; exit 1; }; \
	fi
	@sudo install -m 644 target/wasm32-wasip2/release/libloft.rlib /usr/local/share/loft/wasm32-wasip2/
	@sudo rm -f /usr/local/share/loft/wasm32-wasip2/deps/*.rlib
	@sudo cp target/wasm32-wasip2/release/deps/*.rlib /usr/local/share/loft/wasm32-wasip2/deps/
	@sudo install -d /usr/local/share/loft/wasm32-unknown-unknown/deps
	@sudo install -m 644 target/wasm32-unknown-unknown/release/libloft.rlib /usr/local/share/loft/wasm32-unknown-unknown/
	@sudo rm -f /usr/local/share/loft/wasm32-unknown-unknown/deps/*.rlib
	@sudo cp target/wasm32-unknown-unknown/release/deps/*.rlib /usr/local/share/loft/wasm32-unknown-unknown/deps/
	@sudo chmod -R a+rX /usr/local/share/loft
	@sudo install -m 755 target/release/loft /usr/local/bin/loft
	@smoke="$${TMPDIR:-/tmp}/loft-install-smoke.loft"; \
	printf 'fn main() {\n    println("loft install smoke ok")\n}\n' > "$$smoke"; \
	if ! /usr/local/bin/loft --interpret "$$smoke" >/dev/null 2>"$$smoke.err"; then \
		echo "ERROR: 'make install' left a broken binary<->stdlib pair —"; \
		echo "the installed loft cannot run the installed stdlib:"; \
		sed 's/^/    /' "$$smoke.err"; \
		echo "Fix: rebuild + reinstall as one unit:  make all && make install"; \
		rm -f "$$smoke" "$$smoke.err"; exit 1; \
	fi; \
	rm -f "$$smoke" "$$smoke.err"; \
	echo "install: post-install smoke OK (installed loft runs the installed stdlib)"
uninstall:
	sudo rm -f /usr/local/bin/loft
	sudo rm -rf /usr/local/share/loft

debug:
	RUSTFLAGS=-g RUST_BACKTRACE=1 cargo build -v
	sudo ln -f -s ${PWD}/target/debug/loft /usr/local/bin/loft

# Rebuild every derived artefact the test suite depends on.  Covers
# four classes of stale artefact that each cascade into misleading
# test failures (rustc 1.94→1.96 update on 2026-05-29 surfaced #2):
#   0. The top-level release rlib `target/release/libloft.rlib` —
#      linked by the native code-gen pipeline (P200/P244 tests) —
#      AND the release `target/release/loft` binary.  `--bins` is
#      load-bearing (#304): tests that spawn the release binary by
#      path (tests/viewer_markdown.rs) get a stale-source binary
#      otherwise — `make ci` runs its tests in the debug profile, so
#      nothing else in that chain ever rebuilds the release bin, and
#      the fresh rlib + stale bin pair made the binary build (or
#      validate) native cdylibs against a loft that is not its own.
#      Cargo's fingerprint detects rustc version bumps, so this is
#      a no-op when fresh and a forced rebuild after a toolchain
#      update.
#   1. Sibling cdylibs under lib/*/native/  (loaded via
#      extensions::load_all, linked via --native)
#   2. Test fixture cdylibs under tests/lib/*/native/
#   3. The wasm32-unknown-unknown rlib the html_wasm suite links
#      AND the wasm32-wasip2 rlib the wasm_library_suite uses
#      (only when the target dir already exists, so `make test`
#      doesn't impose the wasm targets on developers who never
#      touch --html / wasip2)
# Cargo is incremental; each step is ~free on a clean tree.
rebuild-native-cdylibs:
	@cargo build --release --lib --bins -q || { \
	  echo "FAIL: top-level libloft.rlib + loft binary rebuild"; exit 1; \
	}
	@for d in lib/*/native tests/lib/*/native; do \
	  [ -f "$$d/Cargo.toml" ] || continue; \
	  (cd "$$d" && cargo build --release -q) || { \
	    echo "FAIL: rebuild $$d"; exit 1; \
	  }; \
	done
	@# Gate on the target's std being INSTALLED, not on the output dir
	@# (which persists across toolchain switches — a fresh `rustup default`
	@# drops the wasm std but leaves target/wasm32-*/, so the old `[ -d … ]`
	@# heuristic hard-failed with a buried E0463).  Installed → rebuild;
	@# stale artefact but std gone → warn with the fix and SKIP; neither → skip.
	@if rustup target list --installed 2>/dev/null | grep -qx wasm32-unknown-unknown; then \
	  cargo build --release --target wasm32-unknown-unknown \
	    --lib --no-default-features --features random -q || { \
	    echo "FAIL: wasm32-unknown-unknown rlib rebuild"; exit 1; \
	  }; \
	elif [ -d target/wasm32-unknown-unknown ]; then \
	  echo "WARN: wasm32-unknown-unknown std not installed — skipping wasm rlib refresh"; \
	  echo "      (stale target/wasm32-unknown-unknown/ present; run: rustup target add wasm32-unknown-unknown)"; \
	fi
	@if rustup target list --installed 2>/dev/null | grep -qx wasm32-wasip2; then \
	  cargo build --release --target wasm32-wasip2 \
	    --lib --no-default-features --features random -q || { \
	    echo "FAIL: wasm32-wasip2 rlib rebuild"; exit 1; \
	  }; \
	elif [ -d target/wasm32-wasip2 ]; then \
	  echo "WARN: wasm32-wasip2 std not installed — skipping wasm rlib refresh"; \
	  echo "      (stale target/wasm32-wasip2/ present; run: rustup target add wasm32-wasip2)"; \
	fi

# Disk-backed scratch for native/wasm test compiles — keeps rustc/rust-lld link
# intermediates AND the loft native binary cache off the /tmp tmpfs (a ~7.5G RAM
# disk on some boxes), which parallel compiles otherwise exhaust → rust-lld dies
# with SIGBUS, surfacing as flaky, unrelated failures.  TMPDIR also redirects
# every std::env::temp_dir() user (cross_mode / exit_codes / html_wasm), and
# scratch_dir() falls back to TMPDIR when LOFT_TMPDIR is unset.  Mirrors the same
# redirect in scripts/find_problems.sh.  MUST be OUTSIDE the repo: a
# `target/`-relative TMPDIR breaks the package/registry tests (they build
# fixtures in temp_dir then package/extract — anything under `target/` is
# excluded, so loft.toml goes missing).  /var/tmp is disk-backed.
TEST_SCRATCH := /var/tmp/loft-test-scratch-$(shell printf '%s' "$(CURDIR)" | cksum | cut -d' ' -f1)
TEST_ENV := TMPDIR=$(TEST_SCRATCH) LOFT_TMPDIR=$(TEST_SCRATCH)

test: clippy rebuild-native-cdylibs
	-rm -f tests/generated/*
	-rm -f tests/dumps/*.txt
	mkdir -p $(TEST_SCRATCH)
	# --release: the loft bytecode interpreter is ~1800x slower in debug
	# mode (debug Rust running an interpreter loop). Release mode keeps
	# the full test suite under a minute instead of 30+ minutes.
	$(TEST_ENV) RUST_BACKTRACE=1 cargo test --release -- --nocapture --test-threads=1 >> result.txt 2>&1

quick: rebuild-native-cdylibs
	mkdir -p $(TEST_SCRATCH)
	$(TEST_ENV) RUST_BACKTRACE=1 cargo test --release -- --nocapture --test-threads=1 > result.txt 2>&1

# make iter TEST=<filter> [TFILE=<test_binary>] [PROFILE=release]
#
# Fast single-test iteration loop.  Defaults to the dev profile (which is
# specially tuned in Cargo.toml: opt-level=1 plus
# debug-assertions=false on the loft package).  Measured here:
#
#   incremental rebuild after touching one file:
#     dev profile     ~2.4s
#     release profile ~26.8s
#
# That's ~11x faster wall-time on the inner debug loop.  Pass
# `PROFILE=release` if you specifically need release behaviour
# (e.g. timing-sensitive parallel tests, or to share cache with
# `make test` / `make ci`).
#
# Cleans tests/dumps/ and tests/generated/ first — these are written
# by every test run and pin codegen output for the LAST run; if you
# alternate profiles or run a different subset, stale fixtures can
# trigger bogus errors (e.g. `attempt to add with overflow` from
# u16::MAX placeholder positions).  `make test` / `make ci` already
# clean them; this matches that behaviour.
#
# TFILE optionally restricts to one test binary — e.g. TFILE=issues
# skips every other test crate's rebuild + run.
#
# Examples:
#   make iter TEST=p197                        # dev profile, all p197*
#   make iter TEST=p194 TFILE=issues           # dev profile, only issues.rs
#   make iter TEST=p197 PROFILE=release        # release profile
#   make iter TEST=introspect TFILE=exit_codes # exit_codes.rs only
iter:
	@if [ -z "$(TEST)" ]; then \
		echo "Usage: make iter TEST=<filter> [TFILE=<test_binary>] [PROFILE=release]"; \
		echo "Examples:"; \
		echo "  make iter TEST=p197"; \
		echo "  make iter TEST=p194 TFILE=issues"; \
		exit 1; \
	fi
	@-rm -f tests/generated/* tests/dumps/*.txt 2>/dev/null
	@TFILE_ARG=$$([ -n "$(TFILE)" ] && echo "--test $(TFILE)" || echo ""); \
	PROFILE_ARG=$$([ "$(PROFILE)" = "release" ] && echo "--release" || echo ""); \
	RUST_BACKTRACE=1 cargo test $$PROFILE_ARG $$TFILE_ARG -- $(TEST) --nocapture

profile:
	RUSTFLAGS=-g cargo build --release >result.txt 2>&1
	flamegraph -o profiler.svg -- target/release/loft auto

# wasm: build the browser bundle (loft.js + loft_bg.wasm under doc/pkg/)
# via wasm-pack.  Uses the `wasm` feature → pulls in wasm-bindgen → the
# resulting wasm has __wbindgen_placeholder__ imports that wasm-bindgen-cli
# replaces during post-processing.  This is the wasm-pack pipeline; it
# is NOT compatible with the `loft --html` pipeline (see `make game` /
# `make wasm-html-test`), which links a no-wasm-feature rlib instead.
#
# CAVEAT: this overwrites target/wasm32-unknown-unknown/release/libloft.rlib
# with the wasm-bindgen variant.  Re-run `make game` (or
# `make wasm-html-test`) afterwards if you need the --html variant back.
wasm:
	$$HOME/.cargo/bin/wasm-pack build --target web --out-dir doc/pkg --release -- --features wasm --no-default-features

# @PLN117 — the THREADED gallery bundle: par() over real Web Worker threads.
# Same shape as `make wasm` but with the wasm-threads recipe (see `wasm-mt` for
# the full-flag-set rationale), output to doc/pkg-mt so it does NOT clobber the
# committed single-threaded doc/pkg (which stays the default — no nightly /
# build-std burden on gallery CI).  To deploy a threaded gallery: build this,
# copy doc/pkg-mt over ./pkg on a COOP/COEP host; the playground/gallery loaders
# start loft's pool automatically when crossOriginIsolated.  Needs the same
# nightly + rust-src toolchain as `wasm-mt`.
gallery-mt:
	RUSTFLAGS='$(WASM_MT_RUSTFLAGS)' \
	rustup run nightly \
	$$HOME/.cargo/bin/wasm-pack build --target web --out-dir doc/pkg-mt --release \
	-- --no-default-features --features wasm-threads -Z build-std=panic_abort,std
	@cp doc/loft-thread.js doc/pkg-mt/
	@echo "Built doc/pkg-mt/ (threaded gallery). Deploy as ./pkg on a COOP/COEP host; serve with 'make serve'."

# gallery: verify-and-rebuild the web gallery end-to-end so it can
# recover from a partially-broken state.  Use this when the browser
# reports errors like "Failed to grow table" (wasm/JS glue mismatch),
# "404 on pkg/loft_bg.wasm" (out-of-tree build), or just after an
# upstream change that invalidates the generated pkg.
#
# Steps (each fails loudly, no silent skips):
#   1. Clean doc/pkg/ entirely so a partial cache cannot hide staleness.
#   2. Check wasm-pack is installed; abort with an actionable message
#      ("cargo install wasm-pack") if not.
#   3. Rebuild the wasm bundle via `make wasm`.
#   4. Verify every file the gallery imports actually exists at the
#      expected path AND is non-empty.
#   5. Verify loft.js and loft_bg.wasm were generated in the SAME run
#      (timestamps within 120s) — a mismatch is the most common source
#      of "failed to grow table" runtime errors.
#   6. Start a transient http.server on a fixed ephemeral port,
#      HEAD-request every asset the gallery loads, fail on non-200.
#   7. Print a one-line "gallery ready" summary with the URL.
#
# After a successful run, `make serve` will work for local browsing.
gallery:
	@echo "  [1/7] cleaning doc/pkg ..."
	@rm -rf doc/pkg
	@echo "  [2/7] checking wasm-pack ..."
	@if [ ! -x "$$HOME/.cargo/bin/wasm-pack" ] && ! command -v wasm-pack >/dev/null 2>&1; then \
		echo "    FAIL: wasm-pack not installed."; \
		echo "    install with: cargo install wasm-pack"; \
		exit 1; \
	fi
	@echo "  [3/7] building wasm bundle ..."
	@$(MAKE) wasm >/tmp/loft_gallery_wasm.log 2>&1 || { \
		echo "    FAIL: wasm-pack build failed — see /tmp/loft_gallery_wasm.log"; \
		tail -20 /tmp/loft_gallery_wasm.log; \
		exit 1; \
	}
	@echo "  [4/7] checking required gallery files ..."
	@missing=0; \
	for f in doc/gallery.html doc/gallery-run.html doc/gallery-examples.js doc/loft-gl.js \
	         doc/pkg/loft.js doc/pkg/loft_bg.wasm doc/pkg/loft.d.ts; do \
		if [ ! -s "$$f" ]; then \
			echo "    FAIL: $$f is missing or empty"; \
			missing=$$((missing + 1)); \
		fi; \
	done; \
	if [ $$missing -gt 0 ]; then exit 1; fi
	@echo "  [5/7] checking wasm/js glue are from the same build ..."
	@js_mtime=$$(stat -c %Y doc/pkg/loft.js); \
	wasm_mtime=$$(stat -c %Y doc/pkg/loft_bg.wasm); \
	delta=$$((wasm_mtime - js_mtime)); \
	delta=$${delta#-}; \
	if [ $$delta -gt 120 ]; then \
		echo "    FAIL: loft.js and loft_bg.wasm timestamps differ by $$delta s"; \
		echo "    One or both is stale — rerun 'make gallery'."; \
		exit 1; \
	fi
	@echo "  [6/7] starting transient http.server and probing assets ..."
	@port=18765; \
	cd doc && python3 -m http.server $$port --bind 127.0.0.1 \
	  >/tmp/loft_gallery_server.log 2>&1 & \
	echo $$! > /tmp/loft_gallery_server.pid; \
	# Give the server a moment to bind the port. \
	for _ in 1 2 3 4 5 6 7 8 9 10; do \
		sleep 0.3; \
		if curl -s -o /dev/null "http://127.0.0.1:$$port/gallery.html"; then break; fi; \
	done; \
	failed=0; \
	for path in /gallery.html /gallery-run.html /gallery-examples.js /loft-gl.js \
	            /pkg/loft.js /pkg/loft_bg.wasm /pkg/loft.d.ts; do \
		code=$$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:$$port$$path"); \
		if [ "$$code" != "200" ]; then \
			echo "    FAIL: http://127.0.0.1:$$port$$path returned $$code"; \
			failed=$$((failed + 1)); \
		fi; \
	done; \
	kill $$(cat /tmp/loft_gallery_server.pid) 2>/dev/null || true; \
	wait $$(cat /tmp/loft_gallery_server.pid) 2>/dev/null || true; \
	rm -f /tmp/loft_gallery_server.pid /tmp/loft_gallery_server.log; \
	if [ $$failed -gt 0 ]; then exit 1; fi
	@# Inject no-cache meta + content-hash version on every local asset
	@# reference so post-deploy browsers fetch fresh.  See
	@# scripts/cache_bust_html.py for rationale.
	@python3 scripts/cache_bust_html.py >/dev/null
	@echo "  [7/7] gallery ready — run 'make serve' and open http://localhost:8000/gallery.html"

# @PLN117 — COOP/COEP so a threaded gallery bundle (`make gallery-mt`) gets
# crossOriginIsolated === true and par() runs on Web Workers.  Harmless for the
# default single-threaded bundle (the gallery is self-contained / same-origin).
# Reuses the cross-origin-isolated static server built for the threaded-wasm
# harness.  Without these headers the gallery still runs — par() just sequential.
serve:
	@echo "Playground: http://localhost:8000/playground.html"
	@echo "Gallery:    http://localhost:8000/gallery.html"
	@echo "(COOP/COEP on — a threaded gallery from 'make gallery-mt' runs par() on Web Workers)"
	python3 tests/wasm/coi-server.py 8000 doc

# ── Branch review viewer (plan-35) ─────────────────────────────
# Serves a branch-aware doc + code review dashboard.
# See doc/claude/plans/35-branch-review-viewer/.
#
# Phase 01 ships INTERPRETER MODE (target/release/loft --interpret).
# Native mode is blocked by P262 (text-call inline-arg codegen quirk)
# + a separate lib/web duplicate-native-fn issue when lib/server is
# pulled transitively.  Phase 07 closeout revisits frozen-binary
# packaging once those blockers close.
#
# view-build: ensure host loft is built; record build provenance.
# view:       refresh state, then run script via loft --interpret.
view-build:
	@echo "  [1/2] building host loft binary ..."
	@cargo build --release -q --lib --bin loft 2>/tmp/loft_view_host.log || { \
	    echo "    FAIL: host cargo build — see /tmp/loft_view_host.log"; \
	    tail -20 /tmp/loft_view_host.log; exit 1; }
	@echo "  [2/2] recording build provenance ..."
	@host_sha=$$(git rev-parse --short HEAD 2>/dev/null || echo "unknown"); \
	{ echo "loft-view phase 01 — interp-mode build"; \
	  echo "Built $$(date -u +%Y-%m-%dT%H:%M:%SZ) against loft commit $$host_sha"; \
	  echo ""; \
	  echo "Native compilation blocked by:"; \
	  echo "  P262 — text-returning calls passed inline get extra & wrap"; \
	  echo "  (lib/web duplicate native fn defs in --native compile)"; \
	  echo ""; \
	  echo "Runs via 'loft --interpret' until those blockers close."; \
	} > tools/viewer/BUILD_NOTES.md
	@echo "loft-view ready: tools/viewer/src/main.loft (interp-mode)"
	@echo "  See tools/viewer/BUILD_NOTES.md for native-mode blockers."

view-refresh:
	@./tools/viewer/refresh.sh

# ── Tracker-tag indexer (plan-37) ───────────────────────────────
# Scans the repo for @P-id / @PLAN-id references, writes
# index/tags.json.  See doc/claude/plans/37-tracker-index/.
# CLI query wrapper (`scripts/idx`) lands in plan-37 phase 01.
index:  ## Refresh index/tags.json via the loft scanner
	@# @PLAN37 phase 07 sub-commits A.5→J: scan.loft is now the
	@# sole canonical scanner; the legacy bash scan.sh + the
	@# `index-bash` / `index-loft` fallback targets were removed
	@# in sub-commit J after sub-commit H (cutover) soaked one
	@# CI cycle on main.  scan.loft writes the JSON-object form
	@# of tags.json to stdout (via `LOFT_INDEX_BUCKETED=1`) and
	@# the summary stats line to stderr — the shell redirect
	@# captures one, the terminal shows the other.  The
	@# `--no-warnings` flag (@P282 close) keeps stdout free of
	@# the loft compiler's warning preamble.
	@if [ ! -x target/release/loft ]; then \
	    echo "host loft binary missing; run: cargo build --release"; exit 1; \
	fi
	@mkdir -p index
	@# `--native-release` (rather than bare `--native`) instructs rustc
	@# to emit `-O` (opt-level=2) and the loft codegen to emit only
	@# reachable functions.  For a hot loop like scan.loft the
	@# difference is dramatic: scan loop 1.7s → 165ms, total 5s → 1.3s
	@# warm-cache.  Cold compile costs ~6s but the per-source cache
	@# (tools/indexer/src/.loft/cache/) survives across runs, so the
	@# everyday `make index` invocation is the warm path.
	@LOFT_INDEX_BUCKETED=1 ./target/release/loft --native-release --no-warnings --lib lib/ \
	    tools/indexer/src/scan.loft > index/tags.json

index-install-hook:
	@./tools/indexer/install-hook.sh

# ── @I81 · @PLN92 feature catalogue sync (strand 3) — sync tooling ──
# The `loft-lang/features` issues are the canonical, self-contained docs; these
# targets keep the in-project shadow one-way: the mirror (doc/features/) that
# agents grep + scan.loft indexes, and the runnable examples (tests/docs/features/)
# that CI runs cross-backend.  See doc/claude/plans/92-feature-catalogue/.
FEATURES_REPO ?= loft-lang/features

features-fetch:  ## Refresh index/features.json from the loft-lang/features tracker (network; gh + jq)
	@gh issue list -R $(FEATURES_REPO) --state all --limit 200 \
	    --json number,title,labels,body \
	  | jq 'sort_by(.number) | map({number, title, kind: ((.labels|map(.name)|map(select(startswith("kind:")))|.[0]) // "kind:unknown" | sub("^kind:";"")), body})' \
	  > index/features.json
	@echo "index/features.json: $$(jq length index/features.json) issues"

features-gen:  ## Regenerate the shadow (doc/features/ + tests/docs/features/) from the snapshot
	@if [ ! -x target/release/loft ]; then \
	    echo "host loft binary missing; run: cargo build --release --bin loft"; exit 1; \
	fi
	@rm -rf doc/features tests/docs/features
	@./target/release/loft --interpret tools/features/gen.loft

features-check: features-gen  ## Drift guard: fail if the committed shadow is stale vs the snapshot
	@out=$$(git status --porcelain -- doc/features tests/docs/features); \
	if [ -n "$$out" ]; then \
	    echo "ERROR: doc/features / tests/docs/features drifted from index/features.json."; \
	    echo "Run 'make features-gen' and commit the result. Offending paths:"; \
	    echo "$$out"; \
	    exit 1; \
	fi
	@echo "features shadow in sync with index/features.json."

api-compat:  ## @PLN102 — check bundled api-surface baselines are still a drop-in (CI: red, non-blocking)
	@cargo build --release --bin loft
	@rc=0; for base in tests/fixtures/api_compat/*.api-baseline; do \
	    src="$${base%.api-baseline}.loft"; \
	    echo "== api-compat: $$src =="; \
	    target/release/loft api-surface --check "$$base" "$$src" || rc=1; \
	done; \
	if [ $$rc -ne 0 ]; then \
	    echo "NOT a drop-in. On an INTENTIONAL change, regenerate: loft api-surface <lib> --emit-baseline > <lib>.api-baseline"; \
	fi; \
	exit $$rc

check-contract-goldens:  ## @PLN102 flip-gate Gate 1+2 — a frozen golden (layout/behaviour) changed ⇒ CONTRACT_VERSION bump (inert at 0; CI: red, non-blocking)
	@scripts/check_contract_goldens.sh --self-test
	@scripts/check_contract_goldens.sh "$${BASE_REF:-origin/main}"

view: view-refresh
	@if [ ! -f tools/viewer/src/main.loft ]; then \
	    echo "loft-view source missing; expected tools/viewer/src/main.loft"; \
	    exit 1; \
	fi
	@if [ ! -x target/release/loft ]; then \
	    echo "host loft binary missing; run: make view-build"; \
	    exit 1; \
	fi
	# Default to --native-release (rustc -O).  Bare --native runs
	# unoptimised generated Rust — for an HTTP server that handles
	# repeated requests, the per-request cost difference is large
	# (10× on hot loops; see PERFORMANCE.md § Open work).  Cold
	# compile is ~6s; cached binary survives across restarts via
	# tools/viewer/src/.loft/cache/.
	# @P274 closed 2026-05-14 (use-after-free in
	# patch_hoisted_returns Pass 2 + text-concat type-dispatch in
	# parse_append_text).
	# `LOFT_VIEW_INTERP=1 make view` falls back to the interpreter
	# (useful when bisecting a native-only regression).
	@if [ -n "$$LOFT_VIEW_INTERP" ]; then \
	    ./target/release/loft --interpret --lib lib/ tools/viewer/src/main.loft; \
	else \
	    ./target/release/loft --native-release --lib lib/ tools/viewer/src/main.loft; \
	fi

# game: rebuild the efficient browser build of Brick Buster from any
# state — clean rebuild of the wasm32-unknown-unknown rlibs + host
# binary + `loft --html`, then publish the resulting self-contained
# HTML to doc/brick-buster.html.  Use when the check-in looks broken
# or after an upstream change that invalidates either the wasm rlib
# or the host tooling.
#
# Steps (each fails loudly, no silent skips):
#   1. Rebuild the host binary so --html and its native_utils helpers
#      are current.
#   2. Ensure the wasm32-unknown-unknown target is installed.
#   3. Rebuild the wasm32-unknown-unknown libloft.rlib + deps via
#      `make wasm-assets`; this is the ingredient `--html` links
#      against and is the single most common source of "W1.1"
#      compile failures.
#   4. Verify libloft.rlib exists for both wasm32 and the host
#      (proc-macros need the host deps dir).
#   5. Run `loft --html doc/brick-buster.html ...25-brick-buster.loft`.
#   6. Sanity-check the output HTML: doctype + loft_start + > 5kB.
#   7. Print the file:// URL so the user can click through.
game:
	@echo "  [1/7] building host binary + libloft.rlib ..."
	@# `--bin loft` alone does not always produce the top-level
	@# libloft.rlib that step 4 requires for proc-macro lookup;
	@# building both explicitly guarantees both artefacts exist.
	@cargo build --release -q --lib --bin loft || { echo "    FAIL: host cargo build"; exit 1; }
	@echo "  [2/7] checking wasm32-unknown-unknown target ..."
	@rustup target list --installed 2>/dev/null | grep -q wasm32-unknown-unknown || { \
	    echo "    FAIL: rustup target not installed"; \
	    echo "    install with: rustup target add wasm32-unknown-unknown"; \
	    exit 1; }
	@echo "  [3/7] rebuilding wasm32-unknown-unknown rlibs ..."
	@cargo build --release -q --target wasm32-unknown-unknown --lib --no-default-features --features random \
	    >/tmp/loft_game_wasm.log 2>&1 || { \
	    echo "    FAIL: wasm rlib build — see /tmp/loft_game_wasm.log"; \
	    tail -20 /tmp/loft_game_wasm.log; exit 1; }
	@echo "  [4/7] verifying libloft.rlib for both targets ..."
	@test -f target/wasm32-unknown-unknown/release/libloft.rlib || { \
	    echo "    FAIL: target/wasm32-unknown-unknown/release/libloft.rlib missing"; exit 1; }
	@test -f target/release/libloft.rlib || { \
	    echo "    FAIL: target/release/libloft.rlib missing (needed for proc-macros)"; exit 1; }
	@echo "  [5/7] compiling Brick Buster to self-contained HTML ..."
	@./target/release/loft --html doc/brick-buster.html \
	    --path "$$(pwd)/" --lib "$$(pwd)/lib/" \
	    tools/brick-buster/25-brick-buster.loft \
	    >/tmp/loft_game_html.log 2>&1 || { \
	    echo "    FAIL: --html compilation — see /tmp/loft_game_html.log"; \
	    tail -30 /tmp/loft_game_html.log; exit 1; }
	@echo "  [6/7] sanity-checking HTML output ..."
	@test -f doc/brick-buster.html || { echo "    FAIL: doc/brick-buster.html not created"; exit 1; }
	@size=$$(stat -c %s doc/brick-buster.html 2>/dev/null || stat -f %z doc/brick-buster.html); \
	if [ $$size -lt 5000 ]; then \
	    echo "    FAIL: doc/brick-buster.html is only $$size bytes (expected > 5000)"; exit 1; \
	fi; \
	grep -q "<!DOCTYPE html>" doc/brick-buster.html || { echo "    FAIL: missing DOCTYPE"; exit 1; }; \
	grep -q "loft_start" doc/brick-buster.html || { echo "    FAIL: missing loft_start entry"; exit 1; }
	@# Integrity gate (@P337): a size/DOCTYPE check is NOT enough — the two
	@# ways this bundle silently breaks are (a) a STOMPED rlib (wasm-bindgen
	@# placeholder imports → won't instantiate) and (b) MISSING wasm-opt
	@# (no asyncify → render loop hangs the tab).  Both pass the size check.
	@# This instantiates the embedded wasm with stub imports and asserts the
	@# import/export shape, so a broken bundle fails the build loudly instead
	@# of shipping.  Requires node (already a dev dependency for wasm tests).
	@if command -v node >/dev/null 2>&1; then \
	    node tools/check_html_bundle.mjs doc/brick-buster.html || exit 1; \
	else \
	    echo "    WARN: node not found — skipping bundle integrity check (install node to enable)"; \
	fi
	@# Inject no-cache meta + content-hash version on every local
	@# asset reference so post-deploy browsers fetch fresh.  See
	@# scripts/cache_bust_html.py for rationale (GitHub Pages doesn't
	@# let us set custom HTTP headers; the only lever is the HTML).
	@python3 scripts/cache_bust_html.py >/dev/null
	@echo "  [7/7] Brick Buster ready."
	@echo ""
	@echo "    Open in your browser:"
	@echo "      file://$$(pwd)/doc/brick-buster.html"
	@echo ""
	@echo "    Or serve locally:"
	@echo "      make serve  →  http://localhost:8000/brick-buster.html"

# crystal-editor: build the stand-alone audience crystal editor
# (tools/audience-demo/crystal_editor.loft) to a self-contained browser
# page doc/crystal-editor.html — the GitHub Pages demo.  Mirrors `make
# game`: host binary + wasm rlib, then `loft --html` (which bundles the
# WebGL backend lib/graphics/js/loft-gl.js), then cache-bust.
crystal-editor:
	@echo "  [1/5] building host binary + libloft.rlib ..."
	@cargo build --release -q --lib --bin loft || { echo "    FAIL: host cargo build"; exit 1; }
	@echo "  [2/5] checking wasm32-unknown-unknown target ..."
	@rustup target list --installed 2>/dev/null | grep -q wasm32-unknown-unknown || { \
	    echo "    FAIL: rustup target not installed"; \
	    echo "    install with: rustup target add wasm32-unknown-unknown"; \
	    exit 1; }
	@echo "  [3/5] rebuilding wasm32-unknown-unknown rlibs ..."
	@cargo build --release -q --target wasm32-unknown-unknown --lib --no-default-features --features random \
	    >/tmp/loft_crystal_wasm.log 2>&1 || { \
	    echo "    FAIL: wasm rlib build — see /tmp/loft_crystal_wasm.log"; \
	    tail -20 /tmp/loft_crystal_wasm.log; exit 1; }
	@echo "  [4/5] compiling Crystal Editor to self-contained HTML ..."
	@./target/release/loft --html doc/crystal-editor.html \
	    --path "$$(pwd)/" --lib "$$(pwd)/lib/" \
	    tools/audience-demo/crystal_editor.loft \
	    >/tmp/loft_crystal_html.log 2>&1 || { \
	    echo "    FAIL: --html compilation — see /tmp/loft_crystal_html.log"; \
	    tail -30 /tmp/loft_crystal_html.log; exit 1; }
	@test -f doc/crystal-editor.html || { echo "    FAIL: doc/crystal-editor.html not created"; exit 1; }
	@size=$$(stat -c %s doc/crystal-editor.html 2>/dev/null || stat -f %z doc/crystal-editor.html); \
	if [ $$size -lt 5000 ]; then \
	    echo "    FAIL: doc/crystal-editor.html is only $$size bytes (expected > 5000)"; exit 1; \
	fi; \
	grep -q "<!DOCTYPE html>" doc/crystal-editor.html || { echo "    FAIL: missing DOCTYPE"; exit 1; }; \
	grep -q "loft_start" doc/crystal-editor.html || { echo "    FAIL: missing loft_start entry"; exit 1; }
	@python3 scripts/cache_bust_html.py >/dev/null
	@echo "  [5/5] Crystal Editor ready."
	@echo ""
	@echo "    Open in your browser:"
	@echo "      file://$$(pwd)/doc/crystal-editor.html"
	@echo ""
	@echo "    Or serve locally:"
	@echo "      make serve  →  http://localhost:8000/crystal-editor.html"

# play: validate everything needed for a native OpenGL run of Brick
# Buster, then launch the game.  Prerequisites checked in order so
# the first missing item fails fast with an actionable message.
play:
	@echo "  [1/5] checking loft binary + libloft.rlib ..."
	@# Build both --lib and --bin so libloft.rlib (consumed by the
	@# /tmp/loft_native.rs compile in step 5) matches the current
	@# transitive dep set.  Building only --bin can leave a stale
	@# libloft.rlib referencing an older rand_core and rustc fails
	@# with E0460 "found possibly newer version of crate `rand_core`".
	@cargo build --release -q --lib --bin loft 2>/tmp/loft_play_host.log || { \
	    echo "    FAIL: host cargo build — see /tmp/loft_play_host.log"; \
	    tail -20 /tmp/loft_play_host.log; exit 1; }
	@echo "  [2/5] checking system GL libraries ..."
	@if command -v pkg-config >/dev/null 2>&1; then \
	    pkg-config --exists gl || { \
	        echo "    FAIL: OpenGL development headers not found"; \
	        echo "    install:  apt install libgl1-mesa-dev  (debian/ubuntu)"; \
	        echo "              dnf install mesa-libGL-devel  (fedora)"; \
	        echo "              brew install mesa             (macos)"; \
	        exit 1; }; \
	else \
	    echo "    note: pkg-config not found; trusting rustc to link GL"; \
	fi
	@echo "  [3/5] building native graphics cdylib ..."
	@# Try once; on E0460 (stale incremental rmeta) auto-clean and retry
	@# once before surfacing the failure.  This is a pure cargo caching
	@# artefact that otherwise presents as "can't find crate / found
	@# possibly newer version of crate X" and blocks the first-time run
	@# on any checkout where the subcrate was built against different
	@# deps than the current workspace.
	@cd lib/graphics/native && cargo build --release -q 2>/tmp/loft_play_graphics.log || { \
	    if grep -qE 'E0460|E0463' /tmp/loft_play_graphics.log; then \
	        echo "    stale incremental build detected, running cargo clean + retry ..."; \
	        cd lib/graphics/native && cargo clean -q >/dev/null 2>&1; \
	        cd lib/graphics/native && cargo build --release -q 2>>/tmp/loft_play_graphics.log || { \
	            echo "    FAIL: lib/graphics/native build after clean — see /tmp/loft_play_graphics.log"; \
	            tail -30 /tmp/loft_play_graphics.log; exit 1; }; \
	    else \
	        echo "    FAIL: lib/graphics/native build — see /tmp/loft_play_graphics.log"; \
	        tail -30 /tmp/loft_play_graphics.log; \
	        echo ""; \
	        echo "    Common causes:"; \
	        echo "      - missing X11 / Wayland dev headers (libx11-dev, libwayland-dev)"; \
	        echo "      - missing GLFW system dependency"; \
	        exit 1; \
	    fi; }
	@test -f lib/graphics/native/target/release/libloft_graphics_native.so \
	    -o -f lib/graphics/native/target/release/libloft_graphics_native.dylib \
	    -o -f lib/graphics/native/target/release/loft_graphics_native.dll || { \
	    echo "    FAIL: native graphics cdylib missing after build"; exit 1; }
	@echo "  [4/5] checking display available ..."
	@if [ -z "$$DISPLAY" ] && [ -z "$$WAYLAND_DISPLAY" ]; then \
	    echo "    FAIL: no \$$DISPLAY or \$$WAYLAND_DISPLAY set"; \
	    echo "    headless? prefix the command with 'xvfb-run -a' or run on a desktop session"; \
	    exit 1; \
	fi
	@echo "  [5/5] launching Brick Buster ..."
	@echo ""
	@echo "    Controls: ←/→ or A/D to move, Space to launch, Esc to quit"
	@echo ""
	@# --native-release (rustc -O) for the game frame loop; bare
	@# --native runs unoptimised generated Rust and burns frame
	@# budget on call-ABI / null-sentinel bookkeeping the optimiser
	@# normally elides.  Cold compile is ~6s; cached binary survives
	@# across runs.
	@./target/release/loft --native-release \
	    --path "$$(pwd)/" --lib "$$(pwd)/lib/" \
	    tools/brick-buster/25-brick-buster.loft

# ── Native Moros editor ────────────────────────────────────────────
#
# make native-editor  — build + run from source.  Same checklist as
#                       `make play` (host loft + graphics cdylib +
#                       display server).  Opens a 1024x768 window
#                       with the 7x7 starter map.
#
# make editor-dist    — package a relocatable `dist/moros-editor/`
#                       directory: the optimised binary, the
#                       loft_graphics_native.so, any fonts / assets,
#                       and `rpath=$$ORIGIN` so the binary finds the
#                       .so without LD_LIBRARY_PATH.  A user can copy
#                       the entire `dist/moros-editor/` dir anywhere
#                       and run `./moros-editor` without having loft
#                       or the graphics cdylib installed.
native-editor:
	@echo "  [1/3] building loft binary + cdylib ..."
	@cargo build --release -q --lib --bin loft 2>/tmp/loft_editor_host.log || { \
	    echo "    FAIL: host cargo build — see /tmp/loft_editor_host.log"; \
	    tail -20 /tmp/loft_editor_host.log; exit 1; }
	@cd lib/graphics/native && cargo build --release -q 2>/tmp/loft_editor_graphics.log || { \
	    echo "    FAIL: lib/graphics/native — see /tmp/loft_editor_graphics.log"; \
	    tail -20 /tmp/loft_editor_graphics.log; exit 1; }
	@echo "  [2/3] checking display available ..."
	@if [ -z "$$DISPLAY" ] && [ -z "$$WAYLAND_DISPLAY" ]; then \
	    echo "    FAIL: no \$$DISPLAY / \$$WAYLAND_DISPLAY set"; \
	    echo "    run on a desktop session or prefix with 'xvfb-run -a'"; \
	    exit 1; \
	fi
	@echo "  [3/3] launching Moros editor ..."
	@echo "    Controls: WASD move / Arrows camera / 1-6 tools / Ctrl-Z undo"
	@echo "              Left-click paint / F5 save / F9 load / F11 fullscreen / Esc quit"
	@# --native-release for the editor's UI / paint frame loop —
	@# same rationale as `make play`.  Cached binary survives across
	@# runs via lib/graphics/examples/.loft/cache/.
	@./target/release/loft --native-release \
	    --path "$$(pwd)/" --lib "$$(pwd)/lib/" \
	    lib/graphics/examples/moros_editor.loft

editor-dist:
	@echo "  [1/5] building loft binary + cdylib (release-optimised) ..."
	@cargo build --release -q --lib --bin loft 2>/tmp/loft_dist_host.log || { \
	    echo "    FAIL: host cargo build — see /tmp/loft_dist_host.log"; \
	    tail -20 /tmp/loft_dist_host.log; exit 1; }
	@cd lib/graphics/native && cargo build --release -q 2>/tmp/loft_dist_graphics.log || { \
	    echo "    FAIL: graphics cdylib — see /tmp/loft_dist_graphics.log"; \
	    tail -20 /tmp/loft_dist_graphics.log; exit 1; }
	@echo "  [2/5] compiling native_editor.loft with --native-release ..."
	@./target/release/loft --native-release \
	    --path "$$(pwd)/" --lib "$$(pwd)/lib/" \
	    --native-emit /tmp/moros_editor_dist.rs \
	    lib/graphics/examples/moros_editor.loft >/dev/null
	@# Find the cached binary — `loft --native` caches into
	@# <script_dir>/.loft/cache/<stem>-<hash>.  The hash changes
	@# with source, so a glob finds the freshest file.
	@CACHED=$$(ls -t lib/graphics/examples/.loft/cache/moros_editor-* 2>/dev/null | head -1); \
	if [ -z "$$CACHED" ]; then \
	    echo "    FAIL: loft --native-release did not produce a cached binary"; \
	    echo "    looked in lib/graphics/examples/.loft/cache/"; \
	    exit 1; \
	fi; \
	echo "    cached binary: $$CACHED"; \
	echo "  [3/5] assembling dist/moros-editor/ ..."; \
	rm -rf dist/moros-editor; \
	mkdir -p dist/moros-editor/assets; \
	cp "$$CACHED" dist/moros-editor/moros-editor; \
	cp lib/graphics/native/target/release/libloft_graphics_native.so \
	    dist/moros-editor/ 2>/dev/null || \
	cp lib/graphics/native/target/release/libloft_graphics_native.dylib \
	    dist/moros-editor/ 2>/dev/null || \
	cp lib/graphics/native/target/release/loft_graphics_native.dll \
	    dist/moros-editor/ 2>/dev/null || { \
	    echo "    FAIL: graphics cdylib artefact missing"; exit 1; }; \
	cp lib/graphics/examples/DejaVuSans-Bold.ttf \
	    dist/moros-editor/assets/; \
	echo "  [4/5] patching rpath = \$$ORIGIN (Linux / macOS) ..."; \
	if command -v patchelf >/dev/null 2>&1; then \
	    patchelf --set-rpath '$$ORIGIN' dist/moros-editor/moros-editor \
	        && echo "    patchelf ok"; \
	elif command -v install_name_tool >/dev/null 2>&1; then \
	    install_name_tool -add_rpath @executable_path \
	        dist/moros-editor/moros-editor 2>/dev/null \
	        && echo "    install_name_tool ok"; \
	else \
	    echo "    note: neither patchelf nor install_name_tool available"; \
	    echo "    binary will require LD_LIBRARY_PATH=./dist/moros-editor at runtime"; \
	fi
	@echo "  [5/5] distributable layout:"
	@find dist/moros-editor -maxdepth 2 -type f -printf "    %p  (%s bytes)\n" 2>/dev/null || \
	    find dist/moros-editor -maxdepth 2 -type f -exec ls -l {} \; | awk '{printf "    %s  (%s bytes)\n", $$9, $$5}'
	@echo ""
	@echo "  Distributable:  dist/moros-editor/"
	@echo "  Run:            ./dist/moros-editor/moros-editor"

# wasm-html-test: run the WASM-runtime safety gate (tests/html_wasm.rs).
#
# Why this target exists:  `make wasm` (wasm-pack pipeline) and
# `loft --html` (Brick Buster pipeline) both write to the same path
# `target/wasm32-unknown-unknown/release/libloft.rlib` but with
# incompatible feature sets.  After `make wasm` the rlib pulls in
# wasm-bindgen and the html_wasm tests fail to instantiate (`Import #1
# module="__wbindgen_placeholder__": module is not an object or
# function`).  This target rebuilds the rlib in the --html shape
# (no `wasm` feature) before invoking the test, so the gate is
# deterministic regardless of what was built last.
#
# RELEASE.md § Safety gate cites this as the WASM-runtime gate.
wasm-html-test:
	@echo "  [1/3] checking wasm32-unknown-unknown target ..."
	@rustup target list --installed 2>/dev/null | grep -q wasm32-unknown-unknown || { \
	    echo "    FAIL: rustup target not installed"; \
	    echo "    install with: rustup target add wasm32-unknown-unknown"; \
	    exit 1; }
	@echo "  [2/3] rebuilding wasm rlib for --html (no wasm feature) ..."
	@cargo build --release -q --target wasm32-unknown-unknown --lib --no-default-features --features random || { \
	    echo "    FAIL: wasm rlib build"; exit 1; }
	@cargo build --release -q --lib --bin loft || { \
	    echo "    FAIL: host binary + libloft.rlib build"; exit 1; }
	@echo "  [3/3] running html_wasm safety gate ..."
	@cargo test --release --test html_wasm

# Browser-side WebGL rendering gate.  Builds doc/brick-buster.html
# (only browser-deployed --html artefact today), loads it in headless
# Chrome + SwiftShader, fails on any JS console.error / exception.
#
# Catches shader-compile regressions that `wasm-html-test` cannot
# (compileShader errors are JS-side; the WASM `loft_start` returns
# cleanly even when every frame fails to draw).  Skips cleanly when
# google-chrome / node / wasm32 toolchain are not installed.
#
# Wired into `cargo test --release` automatically (so `make ship` /
# `make ci` pick it up); this target is for ad-hoc invocation.
test-html-render:
	@# Build the HTML artefact BEFORE the cargo test invocation.
	@# tests/html_render.rs intentionally does NOT auto-build via
	@# `make game` — that would invoke `cargo build` mid-`cargo test`
	@# and race the rustc invocations in tests/native.rs over
	@# target/release/deps/.  Calling `make game` from this target
	@# keeps the build outside the test process.
	@$(MAKE) game >/dev/null
	@cargo test --release --test html_render

clean:
	-rm -rf result.txt tests/dumps/*.txt tests/generated/* pkg target/* perf.data perf.data.old profiler.svg

# Nuke only the wasm32 build trees + the browser bundle.  Run when a
# rustc bump leaves a stale rlib that masks real wasm regressions
# (pre-existing failures silently pass until a full rebuild).  Keep
# this OUT of `make ci` — the fast gate stays fast.  Use before
# `make wasm-html-test` or `make gallery` when you suspect staleness.
clean-wasm:
	-rm -rf target/wasm32-unknown-unknown target/wasm32-wasip2 doc/pkg

# @PLN117 — threaded browser bundle: par() / par_fold over real Web Worker
# threads on loft's own pool (rayon on a SharedArrayBuffer + wasm atomics; the
# same runtime `loft --html` links, see src/wasm_threads.rs).
# Needs the nightly toolchain WITH rust-src (build-std rebuilds std with
# atomics) plus wasm-pack.  --target web is MANDATORY — the worker bootstrap
# imports the generated glue as a module; --target nodejs cannot drive it (and
# node has no Web Worker).  A page must `await init()` then
# `startLoftWorkers(wasm, n, {memory, mainJS})` from loft-thread.js before any
# par; on a host without cross-origin isolation it skips the pool and par()
# falls back to sequential (never breaks — verified in-browser).
# Prove it: python3 tests/wasm/coi-server.py 8799 tests/wasm & then load
# /par-thread-proof.html in a cross-origin-isolated browser.  Design + arcs:
# doc/claude/plans/117-browser-multithreading/.
#
# The link-arg set below is what wasm-bindgen's thread transform requires and
# this toolchain's rustc does NOT auto-emit from +atomics alone: shared +
# imported memory with a maximum, plus lld's synthesized TLS / heap-base
# globals kept as exports.  Drop any one and the bundle silently builds with a
# NON-shared memory — workers then die at runtime with "Memory could not be
# cloned".  (max-memory 1 GiB = 16384 wasm pages.)
WASM_MT_RUSTFLAGS = -C target-feature=+atomics,+bulk-memory,+mutable-globals \
  -C link-arg=--shared-memory -C link-arg=--max-memory=1073741824 \
  -C link-arg=--import-memory -C link-arg=--export=__heap_base \
  -C link-arg=--export=__wasm_init_tls -C link-arg=--export=__tls_size \
  -C link-arg=--export=__tls_align -C link-arg=--export=__tls_base \
  -C link-arg=--export=__stack_pointer

# @PLN117 — prove `par` still computes the right answers in a build WITHOUT the
# threading feature (the shape `make wasm` ships to the browser).  CI only
# `cargo check`s that configuration, which is how a par that returned garbage
# there stayed invisible: the family's native entries were feature-gated out, so
# the interpreter called functions that were not registered.  This runs the
# threading scripts on both builds and diffs — a par must mean the same thing
# with and without threads, only slower.
check-no-threading:
	@CARGO_TARGET_DIR=target-nothread cargo build --release -q --no-default-features --features "mmap random"
	@cargo build --release -q
	@fail=0; for f in tests/scripts/22-threading.loft tests/scripts/22b-par-fold.loft \
	                 tests/scripts/22c-par-sources.loft tests/scripts/22d-par-narrow.loft; do \
	  ./target/release/loft --interpret $$f 2>/dev/null > /tmp/loft-thr.out; \
	  ./target-nothread/release/loft --path $$(pwd)/ --interpret $$f 2>/dev/null > /tmp/loft-nothr.out; \
	  if diff -q /tmp/loft-thr.out /tmp/loft-nothr.out >/dev/null; then echo "  ok   $$f"; \
	  else echo "  FAIL $$f — par differs with and without the threading feature"; fail=1; fi; done; \
	rm -f /tmp/loft-thr.out /tmp/loft-nothr.out; \
	if [ $$fail -eq 0 ]; then echo "PASS: par is identical with and without threads"; fi; exit $$fail

# @PLN117 — every in-browser threading gate, in one command.  Each measures a
# claim rather than assuming it: that `par` really dispatches across Web Worker
# threads, that the shared-memory model still holds, that it scales, that the UI
# stays responsive, and that a `loft --html` page does all of that too — each
# against the value the interpreter produces.  Needs a headless chromium; the
# bundle gates additionally need `make wasm-mt` (they SKIP without it).  The
# runner keeps going after a failing gate and prints one table at the end;
# `scripts/par_gates.sh --ci` is the same run with a SKIP promoted to a failure,
# which is what .github/workflows/browser-threads.yml runs nightly.
par-gates:
	@scripts/par_gates.sh

# @PLN117 — type-check loft's OWN browser thread pool (src/wasm_threads.rs).  It
# is browser-only by nature, so the host `cargo clippy --all-features` never sees
# it; this is where it gets compiled.  Same recipe the `--html` threaded build
# uses, minus the link step.
check-wasm-threads:
	RUSTFLAGS='-Ctarget-feature=+atomics,+bulk-memory,+mutable-globals' \
	rustup run nightly cargo check --lib --target wasm32-unknown-unknown \
	--no-default-features --features "random wasm-native-threads" \
	--target-dir target/loft/html-mt -Zbuild-std=panic_abort,std

wasm-mt:
	RUSTFLAGS='$(WASM_MT_RUSTFLAGS)' \
	rustup run nightly \
	$$HOME/.cargo/bin/wasm-pack build --target web --out-dir tests/wasm/pkg-mt --release \
	-- --no-default-features --features wasm-threads -Z build-std=panic_abort,std
	@cp doc/loft-thread.js tests/wasm/pkg-mt/
	@echo "Built tests/wasm/pkg-mt/ (--target web, threaded, shared memory)."

fill:
	@cargo build --release -q
	@echo "Regenerating src/fill.rs from default/*.loft ..."
	@cargo test --test issues regen_fill_rs -- --ignored --nocapture > /dev/null 2>&1
	@echo "Done. Review with: git diff src/fill.rs"

# List all currently-open P-issues from PROBLEMS.md.  Source of
# truth is the Quick-Reference table; this extracts rows whose
# severity column contains the word "open" or marks the issue
# `(partial)`.  Mirror of the `🔴 Currently Open (fast index)`
# section at the top of the doc — that section uses
# `(partial)` for half-closed issues like P229 (Linux fixed,
# Windows still flaky), and the table writes that as
# `(a) Closed; (b) Open (Windows)` in the severity cell.  The
# regex matches "open" as a whole word so it catches both
# styles without false-positive on `(closed)`.
# `(observation-only)` rows (transient bugs filed once but
# not reproducible) are intentionally omitted — they're a
# watch list, not actionable items.
# `tests/doc_hygiene.rs::problems_open_index_matches_quickref`
# asserts the fast-index and table stay in sync.
problems:
	@awk -F'|' '\
	  /^## Open Issues — Quick Reference/ {flag=1; next} \
	  /^## / && flag {exit} \
	  flag && /^\| [0-9]+ \|/ { \
	    pid = $$2; gsub(/ /, "", pid); \
	    sev = $$4; gsub(/^ +| +$$/, "", sev); \
	    if (tolower(sev) ~ /(^| )open( |[)]|$$)|[(]partial/) { \
	      desc = $$3; gsub(/^ +/, "", desc); \
	      pos = index(desc, "."); \
	      if (pos > 0 && pos < 200) desc = substr(desc, 1, pos); \
	      else if (length(desc) > 180) desc = substr(desc, 1, 180) "..."; \
	      printf "P%-4s  %-65s  %s\n", pid, substr(sev, 1, 65), desc; \
	    } \
	  }' doc/claude/PROBLEMS.md

test-packages:
	@cargo build --release -q
	@failed=0; total=0; \
	for pkg in lib/*/; do \
		if [ ! -f "$$pkg/loft.toml" ]; then continue; fi; \
		if [ ! -d "$$pkg/tests" ]; then continue; fi; \
		pkg_name=$$(basename "$$pkg"); \
		if [ -f "$$pkg/.allow_warnings" ]; then \
			deny_env=""; \
		else \
			deny_env="LOFT_DENY_WARNINGS=1"; \
		fi; \
		for f in "$$pkg"/tests/*.loft; do \
			[ -f "$$f" ] || continue; \
			total=$$((total + 1)); \
			printf "  %-50s" "$$pkg_name/$$(basename $$f)"; \
			out=$$(cd "$$pkg" && env $$deny_env ../../target/release/loft test "$$(basename $$f .loft)" 2>&1); \
			code=$$?; \
			if [ $$code -ne 0 ] || echo "$$out" | grep -q "^Error:\|panicked"; then \
				echo "FAILED"; \
				echo "$$out" | grep -A2 "^Error:\|panicked\|--deny-warnings" | head -8; \
				failed=$$((failed + 1)); \
			else \
				echo "ok"; \
			fi; \
		done; \
	done; \
	echo "$$total package tests, $$failed failed"; \
	if [ $$failed -gt 0 ]; then exit 1; fi

# Phase 6t Tier 2 — Rust integration tests living inside each library's
# `native/tests/`.  These travel with the library when it extracts to a
# chunk repo (the library-ci.yml.example template runs `cargo test
# --release` for any package that ships `native/tests/*.rs`).  In the
# monorepo, this target runs the equivalent so coverage doesn't lapse
# while the library still lives here.
test-package-native-tests:
	@failed=0; total=0; \
	for pkg in lib/*/native; do \
		[ -d "$$pkg/tests" ] || continue; \
		pkg_name=$$(basename $$(dirname "$$pkg")); \
		printf "  %-50s" "$$pkg_name/native: cargo test"; \
		total=$$((total + 1)); \
		out=$$(cd "$$pkg" && cargo test --release 2>&1); \
		code=$$?; \
		if [ $$code -ne 0 ]; then \
			echo "FAILED"; \
			echo "$$out" | grep -E "FAILED|panicked|^---- " | head -20; \
			failed=$$((failed + 1)); \
		else \
			echo "ok"; \
		fi; \
	done; \
	echo "$$total native test crates, $$failed failed"; \
	if [ $$failed -gt 0 ]; then exit 1; fi

# Headless GL example tests — tiered:
#
#   test-gl-smoke    : 3 representative examples, ~20s. Wired into `make ci-full`.
#                      Catches catastrophic regressions (window creation,
#                      Painter2D draw path, scene-graph render path).
#   test-gl-headless : full set (14 today, 26 once P120 lands), ~90-180s.
#                      Run on demand: `make test-gl-headless`. Catches
#                      finer-grained regressions.
#
# Both run lib/graphics/examples/*.loft under Xvfb with the Mesa software
# rasterizer for ~5 seconds each, looking for panics. They catch the
# "appears fixed but isn't" failure mode where a unit-level regression
# test passes but the real GL example panics in actual usage (see
# PROBLEMS.md #120).
#
# An example "passes" if it exits with code 0 (clean exit), 124 (our
# 5-second timeout fired — expected for examples with `for _ in 0..1000000`
# game loops), or 143 (SIGTERM). Anything else is a failure, and any
# `panicked` line in stderr is also a failure regardless of exit code.

# Smoke set — one custom example designed for fast, broad coverage of
# the most-likely-to-regress paths in a single ~5s run. Adding more
# coverage to the smoke set should be done by editing 00-smoke.loft,
# not by adding more files here.
GL_SMOKE := 00-smoke

# Examples currently broken by P120 (Delete on locked store in copy_record).
# P120 fixed — const-param store lock now released at function exit.
# All 27 GL examples pass headless.  Keep variable for future skip needs.
GL_HEADLESS_SKIP :=

# Internal helper: run one loft example under Xvfb. Used by both targets.
# $1 = path to .loft file. Returns 0 on success, sets failed counter via stderr.
define gl_headless_run_one
	name=$$(basename "$(1)" .loft); \
	printf "  %-30s " "$$name"; \
	out=$$(timeout 5 xvfb-run -a -s "-screen 0 800x600x24" \
		./target/release/loft --interpret \
			--path $$(pwd)/ --lib $$(pwd)/lib/ \
			"$(1)" 2>&1); \
	code=$$?; \
	if echo "$$out" | grep -q "panicked"; then \
		echo "FAILED (panic)"; \
		echo "$$out" | grep -A2 "panicked" | head -5; \
		failed=$$((failed + 1)); \
	elif [ $$code -eq 0 ] || [ $$code -eq 124 ] || [ $$code -eq 143 ]; then \
		echo "ok"; \
	else \
		echo "FAILED (exit $$code)"; \
		echo "$$out" | tail -3; \
		failed=$$((failed + 1)); \
	fi
endef

test-gl-smoke:
	@cargo build --release -q
	@if ! command -v xvfb-run >/dev/null 2>&1; then \
		echo "  test-gl-smoke: SKIPPED (xvfb-run not installed; apt-get install xvfb)"; \
		exit 0; \
	fi
	@failed=0; total=0; \
	for name in $(GL_SMOKE); do \
		f="lib/graphics/examples/$$name.loft"; \
		[ -f "$$f" ] || { echo "MISSING: $$f"; failed=$$((failed + 1)); continue; }; \
		total=$$((total + 1)); \
		$(call gl_headless_run_one,$$f); \
	done; \
	echo "$$total smoke-tested, $$failed failed"; \
	if [ $$failed -gt 0 ]; then exit 1; fi

# test-gl-golden: render the smoke test under Xvfb and compare the
# resulting screenshot pixel-for-pixel against tests/golden/00-smoke.png.
# Mesa swrast is deterministic, so any non-zero difference indicates a
# real rendering regression — colour swap, missing texture, layout drift,
# font path failure, etc. The bug found today (gl_load_font sentinel
# mismatch hiding all text textures) would have been caught here on the
# first run after the bug was introduced.
#
# Tolerance: 1% per-pixel fuzz, 0 absolute differences allowed. Adjust
# the AE threshold if anti-aliasing on different platforms produces a
# small but bounded difference.
#
# To accept a deliberate visual change, run `make update-gl-golden`.
test-gl-golden:
	@cargo build --release -q
	@if ! command -v xvfb-run >/dev/null 2>&1; then \
		echo "  test-gl-golden: SKIPPED (xvfb-run not installed)"; \
		exit 0; \
	fi
	@if ! command -v compare >/dev/null 2>&1; then \
		echo "  test-gl-golden: SKIPPED (ImageMagick compare not installed)"; \
		exit 0; \
	fi
	@if [ ! -f tests/golden/00-smoke.png ]; then \
		echo "  test-gl-golden: FAIL — tests/golden/00-smoke.png missing."; \
		echo "  Run 'make update-gl-golden' to create it."; \
		exit 1; \
	fi
	@mkdir -p /tmp/loft_test_render
	@printf "  %-30s " "00-smoke.png vs golden"
	@xvfb-run -a -s "-screen 0 400x300x24" \
		tests/scripts/snap_smoke.sh /tmp/loft_test_render/00-smoke.png \
		>/tmp/loft_golden.log 2>&1; \
	rc=$$?; \
	if [ $$rc -ne 0 ]; then \
		echo "FAIL (snapshot)"; \
		cat /tmp/loft_golden.log; \
		exit 1; \
	fi; \
	diff_count=$$(compare -metric AE -fuzz 1% \
		tests/golden/00-smoke.png \
		/tmp/loft_test_render/00-smoke.png \
		/tmp/loft_test_render/00-smoke-diff.png 2>&1); \
	if [ "$$diff_count" = "0" ]; then \
		echo "ok (0 px differ)"; \
	else \
		echo "FAIL ($$diff_count px differ)"; \
		echo "  Diff written to /tmp/loft_test_render/00-smoke-diff.png"; \
		echo "  If the change is intentional, run: make update-gl-golden"; \
		exit 1; \
	fi

# Regenerate tests/golden/00-smoke.png from the current build. Use after
# an intentional visual change to the smoke test or to a renderer code
# path that affects it.
update-gl-golden:
	@cargo build --release -q
	@if ! command -v xvfb-run >/dev/null 2>&1; then \
		echo "  update-gl-golden: requires xvfb-run"; exit 1; \
	fi
	@mkdir -p tests/golden
	@xvfb-run -a -s "-screen 0 400x300x24" \
		tests/scripts/snap_smoke.sh tests/golden/00-smoke.png
	@echo "  Updated tests/golden/00-smoke.png"
	@echo "  Inspect with: xdg-open tests/golden/00-smoke.png"

test-gl-headless:
	@cargo build --release -q
	@if ! command -v xvfb-run >/dev/null 2>&1; then \
		echo "  test-gl-headless: SKIPPED (xvfb-run not installed; apt-get install xvfb)"; \
		exit 0; \
	fi
	@failed=0; total=0; skipped=0; \
	skip_pattern="$$(echo "$(GL_HEADLESS_SKIP)" | tr ' ' '|')"; \
	for f in lib/graphics/examples/*.loft; do \
		[ -f "$$f" ] || continue; \
		name=$$(basename "$$f" .loft); \
		if echo "$$name" | grep -qE "^($$skip_pattern)$$"; then \
			printf "  %-30s SKIP (PROBLEMS.md P120)\n" "$$name"; \
			skipped=$$((skipped + 1)); \
			continue; \
		fi; \
		total=$$((total + 1)); \
		$(call gl_headless_run_one,$$f); \
	done; \
	echo "$$total tested, $$skipped skipped, $$failed failed"; \
	if [ $$failed -gt 0 ]; then exit 1; fi

ci-miri:  ## @PLAN53: run the loft interpreter under Miri (hard-UB gate). SLOW (~15 min/test).
	@# Mirror of .github/workflows/miri.yml.  Catches alignment / OOB / UAF /
	@# uninitialised / leak UB the homegrown stack_align_guard can't see.
	@# Needs nightly + miri:  rustup toolchain install nightly --component miri
	@# -Zmiri-disable-stacked-borrows gates the HARD memory UB, not the aliasing
	@# model (loft's store layer aliases distinct records by design).  Runs on the
	@# aligned interpreter (the hard-UB-clean configuration).  Add validated tests
	@# to the curated list below as the lever closes more clusters.
	cargo +nightly miri setup
	LOFT_ALIGN=1 LOFT_SLOT_V2=drive \
		MIRIFLAGS='-Zmiri-disable-isolation -Zmiri-disable-stacked-borrows' \
		cargo +nightly miri test --test issues -- --exact \
		p213_struct_field_basic_int

ci:
	@# Fresh header FIRST so result.txt can never be mistaken for a stale
	@# run.  rebuild-native-cdylibs is invoked INSIDE the chain below (not as
	@# an order-only prerequisite) so its output — and any failure — lands in
	@# result.txt; as a prerequisite it ran before this truncation and a
	@# prereq failure (e.g. a missing wasm target after a toolchain switch)
	@# left the OLD result.txt in place, masking the real cause.
	@printf '== make ci | %s | %s ==\n' "$$(rustc --version 2>/dev/null)" "$$(date -u +%FT%TZ)" > result.txt
	-rm -rf tests/generated
	-rm -f /tmp/loft_native_*
	# Some tests (e.g. fill_rs_up_to_date, n2..n10) write into tests/generated
	# directly via generate_code_to without first calling create_dir_all.
	# Recreate the directory so these tests don't fail with NotFound when
	# parallel test scheduling lets them race the helpers that *do* create it.
	mkdir -p tests/generated
	# Mirror of .github/workflows/ci.yml in invocation order so a green
	# local `make ci` predicts a green PR:
	#   1. Format     job → cargo fmt -- --check
	#   2. Clippy     job → cargo clippy -- -D warnings    (no --release,
	#                       no --tests, no --no-default-features — that
	#                       matches the remote runner exactly)
	#                  +  → cargo clippy --all-targets --all-features
	#                       -- -D warnings (added 2026-05-29 to catch
	#                       wasm-feature-only defects that the default
	#                       gate misses: the bin/lib module-cfg mismatch
	#                       on wasm_gl, latent silent-no-op in
	#                       parallel.rs, pedantic-lint regressions in
	#                       src/wasm.rs after rustc updates)
	#   3. Doc hygiene job → scripts/check_doc_drift.sh (blocking since
	#                       2026-05-18 — promoted from non-blocking after
	#                       repeated PR-212 cycles where ignored drift
	#                       surfaced as downstream test failures)
	#   4. Test       job → cargo build --all-targets,
	#                       cargo build --release --target wasm32-wasip2/
	#                         wasm32-unknown-unknown --lib (added
	#                         2026-06-12: the wasm targets are a separate
	#                         COMPILE GATE — a cfg'd fn whose call sites
	#                         aren't cfg'd breaks them while every native
	#                         gate stays green, hidden behind stale rlibs
	#                         until something rebuilds one; bit twice in
	#                         the @PLN18 arc.  ~seconds when warm),
	#                       cargo build --no-default-features
	#                         (to an isolated --target-dir: this strips
	#                          `native-extensions` from libloft.rlib, and
	#                          sharing target/debug/deps/ let it stomp the
	#                          rlib the native tests link → intermittent
	#                          `E0433: cannot find native_call`),
	#                       cargo nextest run --profile ci
	#
	# Drift from what GH runs is the most common cause of "passed local,
	# failed remote" — keep this list short and IDENTICAL to ci.yml.
	# The `Browser build + probe` (gallery) job is intentionally not
	# mirrored here: it requires wasm-pack + node + a clean network and
	# is heavy enough that local devs run `make gallery` separately when
	# touching the wasm bundle.  Other dev-only suites (test-packages,
	# test-gl-smoke, test-gl-golden) live in `make ci-full`.
	mkdir -p $(TEST_SCRATCH) && export $(TEST_ENV) && \
	$(MAKE) rebuild-native-cdylibs >> result.txt 2>&1 && \
	cargo fmt -- --check >> result.txt 2>&1 && \
	cargo clippy -- -D warnings >> result.txt 2>&1 && \
	cargo clippy --all-targets --all-features -- -D warnings >> result.txt 2>&1 && \
	scripts/check_doc_drift.sh >> result.txt 2>&1 && \
	cargo build --all-targets >> result.txt 2>&1 && \
	cargo build --no-default-features --target-dir target/nodefault >> result.txt 2>&1 && \
	cargo build --release --target wasm32-wasip2 --lib --no-default-features --features random >> result.txt 2>&1 && \
	cargo build --release --target wasm32-unknown-unknown --lib --no-default-features --features random >> result.txt 2>&1 && \
	(cargo nextest --version >/dev/null 2>&1 || cargo install cargo-nextest --locked) >> result.txt 2>&1 && \
	cargo nextest run --profile ci >> result.txt 2>&1 && \
	echo 'CI-RESULT: ALL GATES PASSED' >> result.txt || \
	{ echo 'CI-RESULT: FAILED — see the last failing command above in result.txt' >> result.txt; exit 1; }

# Local-only superset of `ci`: same gates plus the development suites
# that are NOT in .github/workflows/ci.yml — package smoke tests and
# the GL/golden visual regression.  Useful before merging an
# infrastructure change that could affect them; not required for the
# remote PR gate.
ci-full: ci
	$(MAKE) test-packages >> result.txt 2>&1 && \
	$(MAKE) test-gl-smoke >> result.txt 2>&1 && \
	$(MAKE) test-gl-golden >> result.txt 2>&1

# Doc hygiene checks — non-blocking.
#
# Runs scripts/check_doc_drift.sh:
#   - paths   : every markdown link to a plan resolves
#   - stale   : retired-feature claims (Type::Long, text_code, .loftc, …)
#   - roadmap : every active plan on ROADMAP at canonical path; no
#               finished/deferred crept in as action items
#   - refs    : no normal docs deep-link into closed/deferred plans
#               (closure-rule enforcement)
#   - time    : (warn) calendar-time projections that should be effort letters
#   - libs    : (warn) lib/<name>/ has loft.toml + README.md
#
# Exit code:
#   0 = clean OR only warnings (time/libs)
#   1 = real drift (paths/stale/roadmap/refs)
#
# Intentionally NOT a `ci` dependency.  Doc drift is a soft failure:
# bad links + closure-rule violations don't break the binary.  Run
# locally before opening a PR; the user makes the call.  If you want
# to gate a PR on doc cleanliness, run `make doc-check` and check
# the exit code yourself, OR wire it into a separate non-blocking
# GitHub Actions job.
doc-check:
	@scripts/check_doc_drift.sh

doc-check-quiet:
	@scripts/check_doc_drift.sh -q

# Regenerate the agent-facing installable-library catalogue from the LIVE
# registry: doc/claude/LIBRARIES.md plus the committed snapshot CI generates
# from (doc/claude/registry-index-snapshot.json).  Run after any registry
# change so an agent in this repo always sees what's already published — and
# never reimplements a registered library.  CI runs the same generator in
# --check mode against the snapshot and fails on drift, so commit both files
# this writes.
libcatalogue:
	@python3 scripts/refresh-unreleased.py   # @PLN112 — origin/main `unreleased` tier (sha-cached)
	@python3 scripts/refresh-loft-release.py # @PLN112 — loft itself (version + binary sha, tag-cached)
	@python3 scripts/refresh-applications.py # @PLN112 — apps: first-party self-described (repo topic+toml) + community issues
	@python3 scripts/gen-library-catalogue.py --refresh-snapshot

# QUALITY Tier 4 #12 — the single pre-push gate.
#
# `ci` optimises for the full automated pipeline: it logs to result.txt,
# runs the GL + packages suites, and only prints on failure.  That's
# great for a remote runner but awkward at the terminal — if you forget
# one of fmt / clippy-default / clippy-ndf / release tests before `git
# push`, the remote CI surfaces it minutes later and the branch sits in
# a partial state.
#
# `ship` is the fast local equivalent: the same four invariants that
# block any push, streamed to the terminal, chained with `&&` so the
# first failure stops the chain and exits non-zero.  The intended
# workflow is `make ship && git push` — if `ship` fails the push
# doesn't happen.
ship:
	cargo fmt --all -- --check && \
	cargo clippy --all-targets --all-features -- -D warnings && \
	cargo clippy --no-default-features --all-targets -- -D warnings && \
	scripts/check_doc_drift.sh && \
	cargo test --release

# The cheap CI gates ONLY (~2-3 min warm): exactly the commands the Format /
# Clippy / Doc-hygiene jobs run, nothing else.  A local pass here removes the
# failure modes that cost a full remote round each ("3-4 runs to get a PR
# green"): fmt drift, a clippy lint the narrower local variants miss
# (`--all-features` is what the CI job adds), a doc-drift ref, and the
# repo-shape guard tests (ship recipe, doc links — seconds to run, and a
# Makefile/doc edit that trips one costs a whole matrix round remotely).
# Use before every push; `make ship` remains the full pre-push gate with tests.
gate:
	cargo fmt --all -- --check && \
	cargo clippy --all-targets --all-features -- -D warnings && \
	scripts/check_doc_drift.sh -q && \
	cargo nextest run --release --test doc_hygiene

run-tests: rebuild-native-cdylibs
	cargo test --release > result.txt 2>&1

clippy:
	cargo fmt -- --check > result.txt 2>&1
	cargo clippy --tests -- -D warnings >> result.txt 2>&1
	cargo check --no-default-features >> result.txt 2>&1

memory:
	cargo test --test vectors -- --nocapture 2>&1 | valgrind --tool=memcheck

last:
	cargo test --package dryopea --test wrap last --release -- --nocapture

meld:
	rustfmt tests/generated/text.rs --edition 2024
	cmp -s tests/generated/text.rs src/text.rs; if [ $$? -eq 1 ]; then meld tests/generated/text.rs src/text.rs; fi
	rustfmt tests/generated/fill.rs --edition 2024
	cmp -s tests/generated/fill.rs src/fill.rs; if [ $$? -eq 1 ]; then meld tests/generated/fill.rs src/fill.rs; fi

generate:
	# cd tests/generated && rustfmt *.rs --edition 2024
	# TODO: target path 'generated/tests/' not present; update when generated workspace is added
	meld tests/generated/ generated/tests/

gtest:
	# TODO: 'generated/' workspace not present; update path when added
	cd generated && cargo clippy --tests -- -W clippy::all -W clippy::cognitive_complexity > result.txt 2>&1
	cd generated && rustfmt tests/*.rs --edition 2024 >> result.txt 2>&1
	cd generated && cargo test -- --nocapture --test-threads=1 >>result.txt 2>&1

bench:
	cargo build --release -q
	bash bench/run_bench.sh --warmup

pdf:
	cargo run --bin gendoc
	typst compile doc/loft-reference.typ doc/loft-reference.pdf

test-native:
	@cargo build --release -q
	@failed=0; \
	for f in tests/docs/*.loft; do \
		printf "  %-45s" "$$f"; \
		out=$$(./target/release/loft --native "$$f" 2>&1); \
		code=$$?; \
		if [ $$code -ne 0 ] || echo "$$out" | grep -q "^Error:\|panicked"; then \
			echo "FAILED"; \
			echo "$$out" | grep -A2 "^Error:\|panicked" | head -5; \
			failed=$$((failed + 1)); \
		else \
			echo "ok"; \
		fi; \
	done; \
	if [ $$failed -gt 0 ]; then \
		echo "$$failed file(s) failed"; \
		exit 1; \
	else \
		echo "All native tests passed."; \
	fi

wasm-assets:
	node tests/wasm/gen-assets.mjs

test-wasm:
	@cargo build --release -q
	@WASMTIME=$$(which wasmtime 2>/dev/null); \
	if [ -z "$$WASMTIME" ] && [ -x "$$HOME/.cargo/bin/wasmtime" ]; then WASMTIME="$$HOME/.cargo/bin/wasmtime"; fi; \
	if [ -z "$$WASMTIME" ] && [ -x "$$HOME/.wasmtime/bin/wasmtime" ]; then WASMTIME="$$HOME/.wasmtime/bin/wasmtime"; fi; \
	if [ -n "$$WASMTIME" ]; then echo "Running wasm tests with wasmtime"; else echo "wasmtime not found — compile-only (install via: cargo install wasmtime-cli)"; fi; \
	failed=0; \
	for f in tests/docs/*.loft tests/scripts/*.loft; do \
		printf "  %-45s" "$$f"; \
		wasm=$$(mktemp /tmp/loft_wasm_XXXXXX.wasm); \
		out=$$(./target/release/loft --native-wasm "$$wasm" "$$f" 2>&1); \
		code=$$?; \
		if [ $$code -ne 0 ]; then \
			rm -f "$$wasm"; \
			echo "FAILED (compile)"; \
			echo "$$out" | head -5; \
			failed=$$((failed + 1)); \
		elif [ -n "$$WASMTIME" ]; then \
			run_out=$$($$WASMTIME --dir . "$$wasm" 2>&1); \
			run_code=$$?; \
			rm -f "$$wasm"; \
			if [ $$run_code -ne 0 ] || echo "$$run_out" | grep -q "^Error:\|panicked"; then \
				echo "FAILED (run)"; \
				echo "$$run_out" | grep -A2 "^Error:\|panicked" | head -5; \
				failed=$$((failed + 1)); \
			else \
				echo "ok"; \
			fi; \
		else \
			rm -f "$$wasm"; \
			echo "ok (compiled)"; \
		fi; \
	done; \
	if [ $$failed -gt 0 ]; then \
		echo "$$failed file(s) failed"; \
		exit 1; \
	else \
		echo "All wasm tests passed."; \
	fi

loft-test:
	@cargo build --bin loft --release -q
	@failed=0; \
	for f in tests/docs/*.loft; do \
		printf "  %-45s" "$$f"; \
		out=$$(./target/release/loft "$$f" 2>&1); \
		code=$$?; \
		if [ $$code -ne 0 ] || echo "$$out" | grep -q "^Error:\|panicked"; then \
			echo "FAILED"; \
			echo "$$out" | grep -A2 "^Error:\|panicked" | head -5; \
			failed=$$((failed + 1)); \
		else \
			echo "ok"; \
		fi; \
	done; \
	if [ $$failed -gt 0 ]; then \
		echo "$$failed file(s) failed"; \
		exit 1; \
	else \
		echo "All loft tests passed."; \
	fi
