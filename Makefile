# Copyright (c) 2022-2025 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later

.PHONY: all check-targets install uninstall debug test quick profile clean fill ci run-tests clippy memory last meld generate gtest pdf bench test-native test-wasm loft-test wasm-assets

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
	sudo install -d /usr/local/share/loft/deps
	sudo install -d /usr/local/share/loft/wasm32-wasip2/deps
	sudo cp -r default /usr/local/share/loft/
	sudo install -m 644 target/release/libloft.rlib /usr/local/share/loft/
	sudo cp target/release/deps/*.rlib /usr/local/share/loft/deps/
	sudo install -m 644 target/wasm32-wasip2/release/libloft.rlib /usr/local/share/loft/wasm32-wasip2/
	sudo cp target/wasm32-wasip2/release/deps/*.rlib /usr/local/share/loft/wasm32-wasip2/deps/
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
	RUST_BACKTRACE=1 cargo test -- --nocapture --test-threads=1 >> result.txt 2>&1

quick:
	RUST_BACKTRACE=1 cargo test --release -- --nocapture --test-threads=1 > result.txt 2>&1

profile:
	RUSTFLAGS=-g cargo build --release >result.txt 2>&1
	flamegraph -o profiler.svg -- target/release/loft auto

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

ci:
	-rm -rf tests/generated
	-rm -f /tmp/loft_native_*
	cargo fmt -- --check > result.txt 2>&1 && \
	cargo clippy --tests -- -D warnings >> result.txt 2>&1 && \
	cargo test >> result.txt 2>&1

run-tests:
	cargo test > result.txt 2>&1

clippy:
	cargo clippy -- -D warnings -W clippy::all > result.txt 2>&1
	cargo clippy --tests -- -D warnings -W clippy::all >> result.txt 2>&1
	rustfmt src/*.rs --edition 2024
	rustfmt tests/*.rs --edition 2024
	cargo run --bin gendoc

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
