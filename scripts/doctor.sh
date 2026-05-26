#!/usr/bin/env bash
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
#
# loft build-toolchain doctor.  Reports the status of every external tool the
# wasm / --html / native pipelines depend on, and prints ENVIRONMENT-SPECIFIC
# install commands for whatever is missing.  Diagnostic only — never installs,
# never fails.  Invoked by `make doctor`.
#
# Why: a machine that compiles the interpreter fine can still silently ship a
# broken browser bundle when the WASM tools are absent (see @P337).  This makes
# the missing pieces — and the exact fix for THIS environment — discoverable.
set -u

# Cargo- and wasmtime-installed tools (wasm-pack, wasmtime) land in these dirs,
# which are on an interactive shell's PATH but often not a non-interactive one.
# Add them so the report reflects what is actually installed, not what this
# particular shell happens to see.
export PATH="$HOME/.cargo/bin:$HOME/.wasmtime/bin:$PATH"

# ── Detect the package manager / install idiom ───────────────────────────────
PM=""
if   command -v apt-get >/dev/null 2>&1; then PM="apt"
elif command -v dnf     >/dev/null 2>&1; then PM="dnf"
elif command -v pacman  >/dev/null 2>&1; then PM="pacman"
elif command -v zypper  >/dev/null 2>&1; then PM="zypper"
elif command -v brew    >/dev/null 2>&1; then PM="brew"
fi

FIXES=()
add_fix() { FIXES+=("$1"); }

# Per-tool install hint, tailored to the detected package manager.
hint() {
  case "$1" in
    binaryen)
      case "$PM" in
        apt)    echo "sudo apt install -y binaryen" ;;
        dnf)    echo "sudo dnf install -y binaryen" ;;
        pacman) echo "sudo pacman -S --needed binaryen" ;;
        zypper) echo "sudo zypper install -y binaryen" ;;
        brew)   echo "brew install binaryen" ;;
        *)      echo "download a release from https://github.com/WebAssembly/binaryen/releases and put 'wasm-opt' on PATH" ;;
      esac ;;
    node)
      case "$PM" in
        apt)    echo "sudo apt install -y nodejs" ;;
        dnf)    echo "sudo dnf install -y nodejs" ;;
        pacman) echo "sudo pacman -S --needed nodejs" ;;
        zypper) echo "sudo zypper install -y nodejs" ;;
        brew)   echo "brew install node" ;;
        *)      echo "install Node.js >= 18 from https://nodejs.org" ;;
      esac ;;
    wasmtime)
      case "$PM" in
        brew)   echo "brew install wasmtime" ;;
        *)      echo "curl https://wasmtime.dev/install.sh -sSf | bash   (or your distro's wasmtime package)" ;;
      esac ;;
    wasm-pack)
      echo "cargo install wasm-pack" ;;
    browser)
      case "$PM" in
        apt)    echo "sudo apt install -y chromium  (or firefox)" ;;
        dnf)    echo "sudo dnf install -y chromium  (or firefox)" ;;
        pacman) echo "sudo pacman -S --needed chromium" ;;
        zypper) echo "sudo zypper install -y chromium" ;;
        brew)   echo "brew install --cask chromium  (or firefox)" ;;
        *)      echo "install Chromium or Firefox" ;;
      esac ;;
    target-uu) echo "rustup target add wasm32-unknown-unknown" ;;
    target-wp) echo "rustup target add wasm32-wasip2" ;;
  esac
}

# ── Report ───────────────────────────────────────────────────────────────────
echo "loft build-toolchain doctor"
echo "==========================="
[ -n "$PM" ] && echo "  (detected package manager: $PM)" || echo "  (no known package manager detected — generic hints below)"
echo ""

row() { printf "  %-22s %s\n" "$1:" "$2"; }

# required core
if v=$(rustc  --version 2>/dev/null); then row rustc "$v"; else row rustc "MISSING (required) — install the Rust toolchain (https://rustup.rs)"; fi
if v=$(cargo  --version 2>/dev/null); then row cargo "$v"; else row cargo "MISSING (required)"; fi

# Plain-language consequence of each missing tool — what the user actually
# loses by skipping it (no jargon).
M_NODE="MISSING — the automatic safety check that catches a broken browser game page before you publish it will be skipped."
M_WASMOPT="MISSING — games you export to the browser will FREEZE the page (the browser tab locks up and never draws). This one is essential if you publish browser games."
M_WASMTIME="MISSING — you won't be able to run or test programs built for the server-style WebAssembly format on this machine."
M_WASMPACK="MISSING — you won't be able to rebuild the in-browser code playground or the example gallery."
M_PYTHON="MISSING — the scripts that prepare published pages (so browsers always fetch the newest version) won't run."
M_BROWSER="none — you can do a basic load check, but you can't actually open an exported game and watch it run to confirm it works."
M_TARGET="MISSING — building anything for the web browser will fail until this is added."

if v=$(node --version 2>/dev/null); then row node "$v"; else
  row node "$M_NODE"; add_fix "node:      $(hint node)"; fi

if v=$(wasm-opt --version 2>/dev/null | head -1); then row "wasm-opt (binaryen)" "$v"; else
  row "wasm-opt (binaryen)" "$M_WASMOPT"; add_fix "wasm-opt:  $(hint binaryen)"; fi

if v=$(wasmtime --version 2>/dev/null); then row wasmtime "$v"; else
  row wasmtime "$M_WASMTIME"; add_fix "wasmtime:  $(hint wasmtime)"; fi

if v=$(wasm-pack --version 2>/dev/null); then row wasm-pack "$v"; else
  row wasm-pack "$M_WASMPACK"; add_fix "wasm-pack: $(hint wasm-pack)"; fi

# wasm-bindgen CLI — informational only (handled automatically when present).
if v=$(wasm-bindgen --version 2>/dev/null); then row wasm-bindgen "$v"; else
  row wasm-bindgen "not present — that's fine, it isn't needed for exporting games to the browser."; fi

if v=$(python3 --version 2>/dev/null); then row python3 "$v"; else row python3 "$M_PYTHON"; fi

# browser — real --html verification
if   v=$(chromium --version 2>/dev/null | head -1); then row browser "$v";
elif v=$(chromium-browser --version 2>/dev/null | head -1); then row browser "$v";
elif v=$(google-chrome --version 2>/dev/null | head -1); then row browser "$v";
elif v=$(firefox --version 2>/dev/null | head -1); then row browser "$v";
else row browser "$M_BROWSER"; add_fix "browser:   $(hint browser)"; fi

echo "  web-browser build support:"
if command -v rustup >/dev/null 2>&1; then
  for t in wasm32-unknown-unknown wasm32-wasip2; do
    if rustup target list --installed 2>/dev/null | grep -q "^$t$"; then
      echo "    [ok]      $t"
    else
      echo "    [MISSING] $t — $M_TARGET"
      [ "$t" = "wasm32-unknown-unknown" ] && add_fix "target:    $(hint target-uu)"
      [ "$t" = "wasm32-wasip2" ]          && add_fix "target:    $(hint target-wp)"
    fi
  done
else
  echo "    rustup not found"
fi

# ── Fix summary ──────────────────────────────────────────────────────────────
echo ""
if [ "${#FIXES[@]}" -eq 0 ]; then
  echo "  All wasm/native toolchain dependencies present."
else
  echo "  To complete the environment, run:"
  for f in "${FIXES[@]}"; do echo "    $f"; done
fi

echo ""
echo "  Heads-up on build order (@P337): if you rebuild the in-browser"
echo "  playground/gallery (\`make wasm\`) and then export a game, the game page"
echo "  can come out broken (blank / won't start).  Use \`make game\` to export —"
echo "  it rebuilds the right pieces in the right order and checks the result"
echo "  before publishing.  Details: doc/claude/WASM.md § Build Toolchain"
echo "  Dependencies."
