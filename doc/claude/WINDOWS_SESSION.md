<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Windows session — what to do when temporary Windows access arrives

Companion to [WINDOWS.md](WINDOWS.md) (the reference doc on Windows compatibility state).  This file is the **session-prep checklist**: priority-ordered investigations, time budget, what to verify FIRST so the session yields signal even if it's short.

## Time budget

A single 1-day session can realistically:
- Verify 2-3 of the G1-G4 gaps (each is "reproduce + capture real error + decide on fix").
- NOT ship all gap fixes; the session's goal is **verified Windows error output**, not closure.

A 2-day session can typically close 1-2 gaps end-to-end (reproduce → fix → un-skip).

Plan for under-2-days first.  Most gap fixes that look "1-day" reveal Windows-toolchain quirks that eat the rest.

## Pre-flight checklist (5-10 min on the laptop)

```powershell
# 1. Rust stable with the MSVC target (default on Windows host)
rustup default stable
rustup target list --installed  # confirm x86_64-pc-windows-msvc

# 2. MSVC build tools — link.exe must be on PATH
where.exe link
# If missing: install "Build Tools for Visual Studio 2022" → "Desktop development with C++"

# 3. cargo-nextest (CI uses this, local repro should match)
cargo install cargo-nextest --locked

# 4. gh CLI for diffing against CI runs
where.exe gh
# If missing: winget install --id GitHub.cli

# 5. Clone + initial build (catches base-toolchain issues immediately)
git clone https://github.com/jjstwerff/loft
cd loft
cargo build --release
cargo build --release --lib   # the unhashed libloft.rlib for --native (reference_native_rlib_rebuild)
```

If any pre-flight step fails, that's the FIRST gap to investigate — base-toolchain breakage gates everything else.

## Priority queue — investigate in this order

### Priority 1 — verify the v2 probe result (5 min if pre-flight clean)

A probe commit (libraries3 `baa9c3e2`, 2026-05-29) temporarily un-ignored `v2_single_client_completes_game` on Windows to surface real `diagnose_listen_failure` output.

```powershell
cd loft
cargo nextest run --test multiplayer_v2 v2_single_client_completes_game
```

Three outcomes:
- **PASSES** → @P229b was incidentally fixed sometime in the last 3 weeks.  Un-ignore all 10 P229b tests across `multiplayer_v{2,3,5}.rs`.  Update G1 in WINDOWS.md.
- **FAILS with port-bind error** in the captured stderr → the 2026-05-21 hypothesis confirms; apply the SO_REUSEADDR / server-binds-:0-itself fix described in WINDOWS.md G1.
- **FAILS with `code: 206` or another spawn error** → same problem space as PR #228 (cmdline overflow).  The spawn-cmdline of the server subprocess is short, so this would point to something else like a missing DLL path or working-dir issue.  Capture the full stderr and investigate.

### Priority 2 — G2 LNK1181 (15-30 min reproduce + decide)

```powershell
cargo run --release -- --check --lib lib tests/fixtures/p310_save_png.loft
```

If LNK1181 fires:
- Confirm whether `build_script_native_lib_dirs` (`src/native_utils.rs`) covers the missing `windows-targets` search dir for the `--check` path.
- The `loft` binary's own invocation works (it adds the dirs); the `--check` STANDALONE codegen path may not.

### Priority 3 — G3 multi-lib transitive-rlib (30-60 min reproduce + decide)

```powershell
cargo run --release -- --check --lib lib tests/fixtures/p310_save_png.loft
```

If `error: crate `ureq` required to be available in rlib format` fires:
- Check `lib/web/native/target/release/deps/libureq-*.rlib` exists.
- Check whether its hash matches what `loft_web.rlib` was linked against.
- Likely fix: lockfile-consistent build resolution, OR restrict `--lib lib` to only the packages the program uses (so a graphics fixture doesn't pull in web's TLS stack).

**Possible PR #228 connection** — if the rustc link command-line for `--native` multi-lib also exceeds Windows' 32 KB `CreateProcessW` limit, the same argfile pattern (`rustc @argfile`) we used in `tests/native.rs::compile_native_job` (PR #228, commit `da1e5e51`) would apply at the production `--native` codegen path.  Worth checking — the link cmdline includes `--extern X=path` for every native dep transitively.  See `src/main.rs` near the `--native` rustc invocation; mirror the argfile pattern from `tests/native.rs:481+` if applicable.

### Priority 4 — G4 `parallel { }` worker stack snapshot (variable effort)

The Linux half of @P229 was fixed 2026-05-10.  The Windows half remains.  Lower priority than G1-G3 because no user has reported a regression from it.

### Priority 5 — opportunistic checks

While the laptop is available, also probe:
- `loft install <pkg>` against a real (non-fixture) registry — @P332 was fixed assuming `LOFT_HOME` plumbing works on Windows; not verified end-to-end.
- `--html` build path on Windows (untested per WINDOWS.md compatibility table — needs `wasm-opt` available).
- Any test marked `STATUS_HEAP_CORRUPTION` previously surfaced (Windows allocator is stricter than Linux's, may catch new OOB bugs).

## What NOT to do

- **Don't try to fix everything.**  The session is finite.  Verified output beats unverified fixes.
- **Don't trust the leading-hypothesis labels in WINDOWS.md / PROBLEMS.md.**  All gap hypotheses are "code review only, unverified" (per the 2026-05-21 note).  PR #228 just proved one of them (P229b implicit assumption about subprocess spawn) was incomplete — the real cause was elsewhere.  Capture the actual Windows error output before adopting any fix.
- **Don't ignore the LNK1181 / G3 connection to PR #228.**  The argfile fix may have a production parallel.  Investigate before re-implementing similar patterns elsewhere.

## After the session

For each gap touched, update:
1. WINDOWS.md — move from `## Known gaps` to `## Previously fixed Windows-only issues` if closed.
2. PROBLEMS.md — close the P-issue with the verified diagnosis.
3. `tests/multiplayer_v{2,3,5}.rs` — un-ignore the affected tests if G1 fixed.
4. `tests/native.rs:960` — drop the LNK1181 runtime-skip branch if G2 fixed.
5. CHANGELOG_TECHNICAL.md — note the Windows-specific fix.

If gaps were verified-but-not-fixed (real error captured, fix needs more time), update the corresponding WINDOWS.md gap section with the captured error message so the next session starts from real data, not hypothesis.

## See also

- [WINDOWS.md](WINDOWS.md) — full reference for compatibility state, gap details, runbooks.
- [PROBLEMS.md](PROBLEMS.md) — @P229 (parallel + multiplayer), @P332 (install), @P333 (lib fixtures).
- [TESTING.md § Open work](TESTING.md#open-work) — @P229b tracking.
- `src/native_utils.rs` — `build_script_native_lib_dirs`, `add_native_extern_flags`.
- `tests/native.rs` lines 481-550 — argfile pattern from PR #228 (potential template for G3 fix).
