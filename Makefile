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
#   make ci         What gets run before every push.
#   make clean      Nuke build artifacts.
#
# More specialised:
#
#   make wasm            Build the wasm-pack bundle that drives the gallery.
#   make install         System-wide install (sudo).
#   make test-gl-golden  Pixel-compare the smoke-test screenshot (Xvfb).
#   make fill            Regenerate src/fill.rs from default/*.loft annotations.
#   make profile         Build with debug symbols + run a flamegraph.
#   make pdf             Rebuild the printable reference PDF.
#
# Every target above is defined as a real rule later in this file.  Scroll
# down to any name to see exactly what it does.
# =========================================================================

.PHONY: all check-targets install uninstall debug test quick profile clean fill ci run-tests clippy memory last meld generate gtest pdf bench test-native test-wasm loft-test wasm-assets test-packages test-gl-headless test-gl-smoke test-gl-golden update-gl-golden serve wasm gallery game play help

# Print the overview at the top of this file.  Useful when you land on a
# fresh checkout and want to know what buttons are available without
# reading a 300-line Makefile.
help:
	@sed -n '/^# ==== What can this Makefile do for you/,/^# ====/p' Makefile \
	  | sed 's/^# \{0,1\}//'

all:
	rustfmt src/*.rs --edition 2024
	RUSTFLAGS=-g cargo build --release

check-targets:
	@missing=""; \
	for target in wasm32-wasip2; do \
		if ! rustup target list --installed | grep -q "^$$target$$"; then \
			missing="$$missing $$target"; \
		fi; \
	done; \
	if [ -n "$$missing" ]; then \
		echo "ERROR: missing rustup target(s):$$missing"; \
		echo "Fix with:$$missing" | sed 's/ / rustup target add /g'; \
		exit 1; \
	fi

install: check-targets all
	cargo build --release --target wasm32-wasip2 --lib --no-default-features --features random
	# W1.1: browser WASM target for --html export
	cargo build --release --target wasm32-unknown-unknown --lib --no-default-features --features random
	# Build library in isolated target dir so deps/ contains exactly one copy
	# of each crate — no binary-only duplicates (e.g. libloading) that cause
	# StableCrateId collisions during native compilation.
	cargo build --release --lib --no-default-features --features mmap,random,threading --target-dir target/install-lib
	sudo install -d /usr/local/share/loft/deps
	sudo install -d /usr/local/share/loft/wasm32-wasip2/deps
	sudo cp -r default /usr/local/share/loft/
	sudo install -m 644 target/install-lib/release/libloft.rlib /usr/local/share/loft/
	sudo rm -f /usr/local/share/loft/deps/*.rlib /usr/local/share/loft/deps/*.so
	sudo cp target/install-lib/release/deps/*.rlib /usr/local/share/loft/deps/
	sudo cp target/install-lib/release/deps/*.so /usr/local/share/loft/deps/ 2>/dev/null || true
	sudo install -m 644 target/wasm32-wasip2/release/libloft.rlib /usr/local/share/loft/wasm32-wasip2/
	sudo rm -f /usr/local/share/loft/wasm32-wasip2/deps/*.rlib
	sudo cp target/wasm32-wasip2/release/deps/*.rlib /usr/local/share/loft/wasm32-wasip2/deps/
	# W1.1: install browser WASM rlib
	sudo install -d /usr/local/share/loft/wasm32-unknown-unknown/deps
	sudo install -m 644 target/wasm32-unknown-unknown/release/libloft.rlib /usr/local/share/loft/wasm32-unknown-unknown/
	sudo rm -f /usr/local/share/loft/wasm32-unknown-unknown/deps/*.rlib
	sudo cp target/wasm32-unknown-unknown/release/deps/*.rlib /usr/local/share/loft/wasm32-unknown-unknown/deps/
	@if ! cmp -s target/release/loft /usr/local/bin/loft; then \
		sudo install -m 755 target/release/loft /usr/local/bin/loft; \
	fi
uninstall:
	sudo rm -f /usr/local/bin/loft
	sudo rm -rf /usr/local/share/loft

debug:
	RUSTFLAGS=-g RUST_BACKTRACE=1 cargo build -v
	sudo ln -f -s ${PWD}/target/debug/loft /usr/local/bin/loft

test: clippy
	-rm -f tests/generated/*
	-rm -f tests/dumps/*.txt
	# --release: the loft bytecode interpreter is ~1800x slower in debug
	# mode (debug Rust running an interpreter loop). Release mode keeps
	# the full test suite under a minute instead of 30+ minutes.
	RUST_BACKTRACE=1 cargo test --release -- --nocapture --test-threads=1 >> result.txt 2>&1

quick:
	RUST_BACKTRACE=1 cargo test --release -- --nocapture --test-threads=1 > result.txt 2>&1

profile:
	RUSTFLAGS=-g cargo build --release >result.txt 2>&1
	flamegraph -o profiler.svg -- target/release/loft auto

wasm:
	$$HOME/.cargo/bin/wasm-pack build --target web --out-dir doc/pkg --release -- --features wasm --no-default-features

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
	for f in doc/gallery.html doc/gallery-examples.js doc/loft-gl.js \
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
	for path in /gallery.html /gallery-examples.js /loft-gl.js \
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
	@echo "  [7/7] gallery ready — run 'make serve' and open http://localhost:8000/gallery.html"

serve:
	@echo "Playground: http://localhost:8000/playground.html"
	@echo "Gallery:    http://localhost:8000/gallery.html"
	cd doc && python3 -m http.server 8000

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
#   5. Run `loft --html doc/brick-buster.html ...brick-buster.loft`.
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
	    lib/graphics/examples/25-brick-buster.loft \
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
	@echo "  [7/7] Brick Buster ready."
	@echo ""
	@echo "    Open in your browser:"
	@echo "      file://$$(pwd)/doc/brick-buster.html"
	@echo ""
	@echo "    Or serve locally:"
	@echo "      make serve  →  http://localhost:8000/brick-buster.html"

# play: validate everything needed for a native OpenGL run of Brick
# Buster, then launch the game.  Prerequisites checked in order so
# the first missing item fails fast with an actionable message.
play:
	@echo "  [1/5] checking loft binary ..."
	@cargo build --release -q --bin loft 2>/tmp/loft_play_host.log || { \
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
	@cd lib/graphics/native && cargo build --release -q 2>/tmp/loft_play_graphics.log || { \
	    echo "    FAIL: lib/graphics/native build — see /tmp/loft_play_graphics.log"; \
	    tail -30 /tmp/loft_play_graphics.log; \
	    echo ""; \
	    echo "    Common causes:"; \
	    echo "      - missing X11 / Wayland dev headers (libx11-dev, libwayland-dev)"; \
	    echo "      - missing GLFW system dependency"; \
	    exit 1; }
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
	@./target/release/loft --native \
	    --path "$$(pwd)/" --lib "$$(pwd)/lib/" \
	    lib/graphics/examples/25-brick-buster.loft

clean:
	-rm -rf result.txt tests/dumps/*.txt tests/generated/* pkg target/* perf.data perf.data.old profiler.svg

wasm-mt:
	RUSTFLAGS='-C target-feature=+atomics,+bulk-memory,+mutable-globals' \
	wasm-pack build --target nodejs --out-dir tests/wasm/pkg-mt \
	-- --features wasm-threads --no-default-features
	@echo "Built tests/wasm/pkg-mt/ — run: node tests/wasm/suite.mjs --threaded 19-threading.loft"

fill:
	@cargo build --release -q
	@echo "Regenerating src/fill.rs from default/*.loft ..."
	@cargo test --test issues regen_fill_rs -- --ignored --nocapture > /dev/null 2>&1
	@echo "Done. Review with: git diff src/fill.rs"

test-packages:
	@cargo build --release -q
	@failed=0; total=0; \
	for pkg in lib/*/; do \
		if [ ! -f "$$pkg/loft.toml" ]; then continue; fi; \
		if [ ! -d "$$pkg/tests" ]; then continue; fi; \
		pkg_name=$$(basename "$$pkg"); \
		for f in "$$pkg"/tests/*.loft; do \
			[ -f "$$f" ] || continue; \
			total=$$((total + 1)); \
			printf "  %-50s" "$$pkg_name/$$(basename $$f)"; \
			out=$$(cd "$$pkg" && ../../target/release/loft test "$$(basename $$f .loft)" 2>&1); \
			code=$$?; \
			if [ $$code -ne 0 ] || echo "$$out" | grep -q "^Error:\|panicked"; then \
				echo "FAILED"; \
				echo "$$out" | grep -A2 "^Error:\|panicked" | head -5; \
				failed=$$((failed + 1)); \
			else \
				echo "ok"; \
			fi; \
		done; \
	done; \
	echo "$$total package tests, $$failed failed"; \
	if [ $$failed -gt 0 ]; then exit 1; fi

# Headless GL example tests — tiered:
#
#   test-gl-smoke    : 3 representative examples, ~20s. Wired into `make ci`.
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

ci:
	-rm -rf tests/generated
	-rm -f /tmp/loft_native_*
	# Some tests (e.g. fill_rs_up_to_date, n2..n10) write into tests/generated
	# directly via generate_code_to without first calling create_dir_all.
	# Recreate the directory so these tests don't fail with NotFound when
	# parallel test scheduling lets them race the helpers that *do* create it.
	mkdir -p tests/generated
	# --release on cargo test: see comment on the `test` target — debug mode
	# pushes the suite from ~1 minute to ~30 minutes because the loft
	# bytecode interpreter is dominated by debug Rust overhead.
	cargo fmt -- --check > result.txt 2>&1 && \
	cargo clippy --tests -- -D warnings >> result.txt 2>&1 && \
	cargo check --no-default-features >> result.txt 2>&1 && \
	cargo test --release >> result.txt 2>&1 && \
	$(MAKE) test-packages >> result.txt 2>&1 && \
	$(MAKE) test-gl-smoke >> result.txt 2>&1 && \
	$(MAKE) test-gl-golden >> result.txt 2>&1

run-tests:
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
