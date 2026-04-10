
# Release Planning

## What each milestone means

**0.8.4 — Awesome Breakout.**
The single-player Breakout demo (`lib/graphics/examples/25-breakout.loft`)
becomes a game someone would actually want to share with a friend: audio,
multiple hand-designed levels, polished art, screen shake, pause menu, high
scores, and a single-file HTML export hosted on itch.io. The headline
language fix is **P122** (free struct/vector temporaries at end-of-loop)
which removes the bitmasks-and-raw-floats workaround pattern from the game
code. Audience: anyone who can click a link.

**0.8.5 — Working Moros editor.**
The Moros hex RPG scene editor runs end-to-end in the browser: load a map,
paint hexes, place walls and items, see a live 3D preview, export to GLB.
Web only — multiplayer comes later. Audience: tabletop RPG hobbyists who
want to design dungeons and ship them as static files.

**0.9.0 — Fully working loft language.**
The language is feature-complete, well-documented, and tooling-friendly.
PROBLEMS.md has zero "appears fixed but unverified" entries. There's a
REPL, a VS Code extension, decent error recovery, and `loft.lock` for
reproducible builds. The four issues today's release-mode switch flagged
as "appears fixed" (P117, P120, P121, P124, P127) are definitively closed.
Audience: developers who want to write loft as a real language, not just
a game scripting tool.

**1.0.0 — Totally sure everything works.**
1.0.0 is the **stability contract**: any program valid on 1.0.0 compiles
and runs identically on any 1.0.x or 1.x.0 release. The contract covers:
- The core language surface (syntax, type system, documented stdlib API, CLI flags).
- The public IDE API (WASM `compileAndRun` / `getSymbols` JS interface).
- The interpreter does not panic or silently produce wrong results.
- A user can write, run, and share a real program — from the terminal or the browser.

The Web IDE (W1–W6), the multiplayer stack (SRV.*, GC.*), and the scene
scripting layer (SC.*) all ship in 1.0.0. Plus the **stability gate** in
ROADMAP.md: valgrind clean, four-platform binaries, no `**High**` open
issues, no shortcuts. See [ROADMAP.md § 1.0.0](ROADMAP.md) for the full
checklist.

---

## Gate Items — MUST for 1.0

These block a 1.0 release because they cause panics on valid programs, ship
incorrect public identity, or leave public keywords in a permanently-broken
state.

Completed historical gate items (T0-1 through T0-7, T1-5, PROBLEMS #10,
#37–#40, A4 pre-gate, Cargo.toml, README, CHANGELOG, CI pipeline, R1) are
recorded in CHANGELOG.md.

**Open as of 2026-04-10:**

- **P122** — store leak in tight game loops. Currently worked around in
  `25-breakout.loft` with bitmasks and raw-float collision APIs. Slated
  for 0.8.4.
- **P117 / P120 / P121 / P124** — flagged `⚠️ Appears fixed but unverified`
  in PROBLEMS.md. Regression-guard tests pass; the original symptoms have
  not been re-validated under their original conditions. Slated for 0.9.0.
- **P127** — file-scope vector constants leak Var() refs into calling
  functions. Has a `#[ignore]`d reproducer test. Slated for 0.9.0.
- **Stability gate** — see ROADMAP.md § 1.0.0 for the full hands-on
  checklist (valgrind, 4-platform binaries, INCONSISTENCIES.md sweep).

---

## Nice-to-Have for 1.0

Include if bandwidth exists before tagging 1.0; ship without if they push the date out significantly.

| Item | Value | Effort |
|---|---|---|
| **T1-2** wildcard imports (`use mylib::*`) | Genuine friction removal; medium payoff | Medium |
| **T1-4** match expressions | Largest language feature gap; makes language feel complete | High |
| **T2-0** code formatter (`loft --format`) | Professional tooling polish; zero correctness risk | Small–Medium |
| HTML reference on GitHub Pages | Users can read docs without cloning the repo | Small (CI step) |
| Pre-built release binaries | Users can install without Rust toolchain | Small (GitHub Actions matrix) |

If T1-4 ships in 1.0, update `doc/09-enum.html` and add `doc/21-match.html`.
If T1-4 does not ship in 1.0, INCONSISTENCY #6 must be prominently documented as a known limitation in CHANGELOG.md and the HTML reference.

---

## Items by milestone

### 0.8.4 gate items

| Item | Notes |
|---|---|
| **P122** store leak in game loops | Free struct/vector temps at end of loop iteration. Today's breakout has to use bitmasks + raw-float collision APIs to dodge this. **Headline language fix for 0.8.4.** |
| G3 tilemap rendering | Lets BK.3 levels be data instead of code |
| G5 audio sound effects | Brick hit, paddle bounce, pickup chimes |
| G6 background music | With volume mix during play |
| W1.1 single-file HTML export | `loft --html game.loft` → one file |
| BK.1–BK.8 game polish | Audio, levels, screen shake, pause menu, title/game-over screens, high scores, art pass |
| G7.P playable Breakout on itch.io | The actual ship target |

### 0.8.5 gate items

| Item | Notes |
|---|---|
| MO.1a–MO.6 moros backend | Data model, palette types, JSON I/O, paint/wall/item ops, undo/redo, slope tool, stencil stamping |
| MO.C1–MO.E5 moros JS frontend | Hex canvas, tools, palettes, undo, localStorage |
| MO.7a–MO.10 moros 3D renderer | Hex geometry, wall slabs, stairs, camera, hex picking, GLB export |
| MO.W1, MO.W2 WASM builds | Editor and renderer compiled for the browser |
| MO.12a–MO.12c WASM↔JS wiring | Live 3D preview panel + GLB export button |
| MO.P moros editor on GH Pages | The actual ship target |

### 0.9.0 gate items

| Item | Notes |
|---|---|
| L1 error recovery | Cascading errors after one bad token; high UX impact |
| P2 REPL / interactive mode | No browser IDE yet (that's 1.0.0); a terminal REPL is the answer for now |
| W-warn developer warnings | Clippy-inspired diagnostics |
| AOT auto-compile libraries | Native shared libs without manual `cargo build` |
| C52 stdlib name clash + `std::` prefix | Naming hygiene |
| C53 match arms with library enums + bare names | Match ergonomics |
| **P127** file-scope vector constants | Already has a reproducer test (`#[ignore]`d); needs Var-index remapping fix |
| **Verify P117** | Re-test the original `file()` pattern with `LOFT_STORES=warn` |
| **Verify P120** | Re-run the full GL example suite end-to-end on a display |
| **Verify P121** | Debug-build valgrind pass over `tests/scripts/50-tuples.loft` |
| **Verify P124** | `--native-emit` inspection of generated Rust |
| SH.1, SH.2 syntax highlighting | TextMate grammar + VS Code extension |
| DX.1, DX.2 quick-start examples + CI | `examples/` directory + CI workflow polish |
| PKG.7 lock file | Reproducible builds |
| FFI.1–FFI.4 native extension docs | Generic marshaller, cdylib loader, docs |

### 1.0.0 gate items (on top of 0.9.0)

| Item | Notes |
|---|---|
| W2 editor shell | Visible IDE |
| W3 symbol navigation | Go-to-definition, find-usages |
| W4 multi-file projects | IndexedDB persistence |
| W5 docs/examples browser | Integrated documentation |
| W6 export/import + PWA | Offline support; closes the loop |
| SC.1–SC.6 scene scripting | Hex enter/exit/interact hooks; in-browser hot-reload |
| SRV.1–SRV.G server library | HTTP routing, WebSockets, auth, ACME, game loop |
| GC.1–GC.6 game client library | WebSocket protocol, lobby, prediction, WASM script loading |
| **Stability gate** | See ROADMAP.md § 1.0.0 — valgrind clean, 4-platform binaries, zero open `**High**` issues, hands-on testing |

### Explicitly 1.1+

| Item | Notes |
|---|---|
| A2 logger production mode | Low user impact until logger is widely used |
| A4 spacial<T> full implementation | After pre-gate added in 0.8.0 |
| A5 closure capture | Very high effort; depends on P1 |
| C57 route decorator syntax | `@get` / `@post` / `@ws` annotations |
| W1.14 WASM Tier 2 | Web Worker pool + `par()` parallelism |

---

## Open Inconsistencies for 1.0

Of the 6 still-open entries in INCONSISTENCIES.md, none are hard blockers, but the following need documentation coverage before 1.0:

| Entry | Action |
|---|---|
| #6 — plain enums cannot have methods | Document as known limitation if T1-4 (match) is deferred; resolved by T1-4 if included |
| #10 — sizeof(u8) returns 4 | Document as accepted behaviour in LOFT.md (compiler minimum alignment) |
| Others | Verify each is documented in LOFT.md / INCONSISTENCIES.md; no code change needed |

---

## Project Structure Changes

### For 1.0 — no crate split needed

The current single-crate layout is correct for the project's scale.  A Cargo workspace split is warranted only when W1 (WASM) starts, so that the `loft-core` library can use `crate-type = ["cdylib","rlib"]` without affecting the CLI binary.

### Cargo.toml changes before 1.0

```toml
[package]
name        = "loft"          # ✓ done 2026-03-15
version     = "1.0.0"             # bump at release
description = "loft — interpreter for the loft scripting language"  # ✓ done 2026-03-15
homepage    = "https://github.com/jjstwerff/loft"  # ✓ done 2026-03-15
repository  = "https://github.com/jjstwerff/loft"  # ✓ done 2026-03-15
keywords    = ["language", "interpreter", "scripting"]  # ✓ done 2026-03-15
categories  = ["command-line-utilities", "compilers"]   # ✓ done 2026-03-15
```

**Note:** `rand_core` and `rand_pcg` are actively used in `src/native.rs` for random number generation — do **not** remove them.  The earlier claim that they were unused was wrong.

**Note on renaming to "loft":** ~~Do it now.~~  **Done 2026-03-15.**  Renaming was free because the package had not yet been published to crates.io.

### Future workspace layout (for W1)

```
Cargo.toml                  (workspace root)
loft-core/              (Cargo.toml: crate-type = ["cdylib","rlib"])
  src/
loft-cli/               (Cargo.toml: [[bin]])
  src/main.rs
loft-gendoc/            (Cargo.toml: [[bin]])
  src/gendoc.rs
default/                    (standard library .loft files)
tests/
doc/
ide/                        (web IDE — added at W1)
```

---

## No Automated Releases

**Releases must never be created or triggered automatically.**  Every release
requires a human validation phase (the checklist below) that cannot be scripted:
hands-on testing of pre-built binaries on each platform, review of the CHANGELOG,
and a deliberate decision to tag and publish.

Do not push release tags, trigger release workflows, draft GitHub Releases, or
run `cargo publish` programmatically.  Always wait for the owner to do this
manually after completing the validation checklist below.

---

## Pre-Release Documentation Review

Run these steps before tagging a release.  They are manual; treat each as a gate item.

### 1 — Audit doc/claude/ for stale problem documentation

- Open PROBLEMS.md: every bug entry there should either be open or clearly crossed out / labelled FIXED with the fix date.  Remove entries that are fixed and already recorded in CHANGELOG.md.
- Open PLANNING.md: every item should be open.  Done items must have been removed (not marked done in-place) before this release.
- Open project_status.md in memory/: verify it reflects current state.

### 2 — Verify code links in doc/claude/

Walk every file in `doc/claude/` looking for references of the form `src/foo.rs`, `src/foo/bar.rs`, function names, struct names, or opcode names.  For each:
- Confirm the file/symbol still exists at that path/name.
- Update any that have moved or been renamed.

Helpful command: `grep -rn 'src/' doc/claude/` and cross-check against `ls src/`.

### 3 — Verify doc/claude/ discoverability

- Every file in `doc/claude/` must be reachable from at least one other file or from the MEMORY.md index.
- Files that are only referenced from MEMORY.md should still link to at least one sibling document.
- Orphaned files (nothing links to them) must be added to an existing doc or removed.

### 4 — Compact verbose sections

Read through any doc/claude/ file that has grown since the previous release and identify passages that are longer than necessary (e.g. multi-paragraph context that can be reduced to a bullet list, repeated caveats, implementation notes already captured in CHANGELOG.md).  Shorten these in place.

### 5 — Validate user documentation against this release

For each feature and bug-fix entry in CHANGELOG.md under `[Unreleased]`:
- Find the corresponding section in the HTML reference (a file in `tests/docs/*.loft` or `doc/`).
- Confirm the user-visible behaviour is correctly described.
- If the feature has no user documentation, add it (either a new `.loft` example or an update to an existing one).

### 6 — Validate DEVELOPERS.md caveats and language-comparison pages

- **`doc/DEVELOPERS.md`**: re-read the compiler pipeline description and all "caveat" or "known limitation" callouts.  Update any that are stale relative to source changes in this release.
- **`doc/00-vs-rust.html`** and **`doc/00-vs-python.html`**: verify that the claims in each comparison table remain accurate for the current language surface (null safety, type inference, collection API, etc.).  Update any cell that no longer holds.

### 7 — Validate user documentation topic flow

- Open `doc/` and list all `NN-*.html` files in order.
- Read the first sentence of each page and verify the sequencing makes sense for a reader progressing top-to-bottom (introductory concepts before advanced ones).
- If a topic added in this release landed at the end of the sequence but logically belongs earlier, renumber and update all cross-links.

### 8 — Validate coding standards and clean up clippy suppressions

```bash
cargo clippy -- -D warnings
```

All warnings must be errors-free.  Additionally, review every `#[allow(clippy::...)]`
annotation in the codebase and attempt to remove it by fixing the underlying code:

```bash
grep -rn "#\[allow(clippy::" src/
```

For each suppression found:
- If the function has been refactored or shortened since the annotation was added, remove
  the `#[allow]` and verify clippy still passes.
- If the suppression covers a genuine structural constraint (e.g. a dispatch function that
  cannot be split without losing clarity), keep it and add a brief comment explaining why.

The goal is to keep suppressions intentional and minimal, not to accumulate them as a
release-over-release debt.

### 9 — Generate HTML and PDF

```sh
# Regenerate HTML reference
cargo run --bin gendoc

# Compile PDF
typst compile doc/loft-reference.typ
```

Verify that `gendoc` completes without warnings and that the generated HTML files look correct in a browser.  Attach `loft-reference.pdf` to the GitHub release.

---

## Release Artifacts Checklist

| Artifact | Required | How |
|---|---|---|
| GitHub release tag `v1.0.0` | Yes | `git tag v1.0.0` |
| Linux static binary (`x86_64-unknown-linux-musl`) | Yes | GitHub Actions + `cross` |
| macOS Intel binary (`x86_64-apple-darwin`) | Yes | GitHub Actions matrix |
| macOS ARM binary (`aarch64-apple-darwin`) | Yes | GitHub Actions matrix |
| Windows binary (`x86_64-pc-windows-msvc`) | Recommended | GitHub Actions matrix |
| `loft-reference.pdf` attached to release | Yes | `typst compile doc/loft-reference.typ` |
| HTML docs on GitHub Pages | Recommended | `cargo run --bin gendoc` → `gh-pages` branch (automated in release.yml) |
| crates.io publish as `loft` | Recommended | `cargo publish` (automated in release.yml via `CARGO_REGISTRY_TOKEN`) |
| `loft.1` man page | Optional | Generate from README with `pandoc` |

---

## Post-1.0.0 Versioning Policy

**Semantic versioning with a roughly monthly release cadence:**

- **1.0.x patch** — bug fixes only; no new language features; no behaviour changes; always backward-compatible.  Example: fix a crash found after 1.0.0 ships.
- **1.x.0 minor** — new language features that are strictly additive (new syntax, new stdlib functions, new CLI flags, new IDE capabilities).  Any program valid on 1.0.0 must compile and run identically on 1.x.0.  Candidates: P2 (REPL), A5 (closures), A7 (native extensions), Tier N (native codegen).
- **2.0** — reserved for breaking language changes.  Not expected in the near term.

The stability guarantee applies to the **loft language surface** (syntax, type system, documented stdlib, CLI flags) and the **public IDE API** (`compileAndRun` / `getSymbols` JS interface).  The Rust library API (`lib.rs`) is not a public stable API until explicitly stabilised.

---

## See also
- [PLANNING.md](PLANNING.md) — Priority-ordered enhancement backlog; source for gate-item IDs
- [ROADMAP.md](ROADMAP.md) — Items grouped by milestone with effort estimates
- [DEVELOPMENT.md](DEVELOPMENT.md) — Branch naming, commit sequence, and CI workflow
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — All known inconsistencies must be resolved or accepted before 1.0.0
