
# Release Planning

## What this file is — and isn't

This file answers one question: **what must be true before we tag
and publish a release of the loft language?**  Every line below
is a gate.  If an item here is still open on release day, the
release slips.  If an item you think matters is not here, it does
not block a release (and probably belongs in
[PLANNING.md](PLANNING.md) or [ROADMAP.md](ROADMAP.md) instead).

RELEASE.md is the **ship checklist**.  The full project backlog,
priorities, and ambitions live elsewhere:

| File | Scope | Question it answers |
|---|---|---|
| **RELEASE.md** (this file) | Ship checklist | "What must be true before we can publish?" |
| **[ROADMAP.md](ROADMAP.md)** | Things we want to do, grouped by milestone | "What's the arc of work for the project, in what order?" |
| **[PLANNING.md](PLANNING.md)** | Priority-ordered backlog, all features | "What's the next best thing to pick up?" |
| **[PROBLEMS.md](PROBLEMS.md)** | Known bugs with severity | "What's broken today?" |
| **[QUALITY.md](QUALITY.md)** | Open programmer-biting issues and active sprints | "Which open issues bite users, and what are we actively working on?" |

RELEASE.md only cites items from those four files — it doesn't
define new work, it promotes existing work to a "must close before
publish" status.  When a ROADMAP.md item becomes a release blocker,
it gets a RELEASE.md row.  When it ships, the RELEASE.md row is
crossed out (the underlying item stays in its home file with its
fix date).

Demo applications (Brick Buster, Moros editor, the Web IDE shell,
the server / game-client libraries, and the scene scripting layer)
follow their own lifecycle and are deliberately out of scope here
— they can ship on their own cadence without gating the language
releases they depend on.  Their individual backlogs live in
[PLANNING.md](PLANNING.md) and [ROADMAP.md](ROADMAP.md).

## What each milestone means

**0.9.0 — Fully working loft language.**
The language is feature-complete, well-documented, and tooling-friendly.
PROBLEMS.md has zero "appears fixed but unverified" entries and no
open compiler-correctness bugs.  A REPL and decent error recovery
ship.  Audience: developers who want to write loft as a real language.

**1.0.0 — Stability contract.**
1.0.0 is the stability contract: any program valid on 1.0.0 compiles
and runs identically on any 1.0.x or 1.x.0 release.  The contract
covers:
- The core language surface (syntax, type system, documented stdlib API, CLI flags).
- The public IDE API (WASM `compileAndRun` / `getSymbols` JS interface).
- A user can write, run, and share a real program — from the terminal or the browser.

Safety (no crashes, no memory corruption, no leaks) is NOT a 1.0
addition — it is the floor for every release, tracked under the
[Safety gate](#safety-gate--blocks-every-release) below.  1.0.0
additionally requires the four-platform-binary stability gate
and a full INCONSISTENCIES.md sweep; see
[ROADMAP.md § 1.0.0](ROADMAP.md).

---

## Safety gate — blocks EVERY release

**We do not ship broken builds.  Ever.**  The items below block
every tag from the next patch release onward, not just 1.0.  A
release that crashes, corrupts memory, or leaks per iteration is
not a release — it's a bug report on a schedule.  If a safety
blocker is open on release day, the release slips.  There is no
"we'll fix it next version" for crashes and leaks.

This bar applies to patch releases, minor releases, and major
releases alike.  It applies whether the target is 0.8.4 or 1.0.0.
A "quick fix" tag that closes one bug but leaves another open is
still a broken build and still gets blocked.

### 0.8.4 progress

**2026-04-14:** tag deferred — safety gate caught P54
chained-call leak (`json_*().method()` leaks temporary store).

**2026-04-25 (dep-fix-sprint):** dep-inference fix landed.
Two changes:
1. Parser (`src/parser/definitions.rs`): native methods
   returning same struct-enum as `self` now carry `dep=[0]`
   (borrow from self).  Constructors (no self) keep `dep=[]`.
2. Scope lift (`src/scopes.rs::inline_struct_return`): native
   struct-enum constructors (empty dep) are lifted to
   temporaries and freed at scope exit.

Result: **79 previously-ignored P54/Q4 leak tests un-ignored
and passing**.  Ignored count in `issues.rs` dropped from 89
to 6 (maintenance, B2/B3 match crash, B5 recursive, B7
character-interpolation, P136 harness, step-6 by design).

**Remaining blockers for 0.8.4 tag:**
- WASM-build + WASM-runtime gates — both verified green
  (run via `make wasm-html-test` to avoid the rlib-feature collision)
- Crash bugs: none (B2-runtime, B3, B5, B7, P136 all closed)
- Zero-leak gate (wrap-suite scripts 42/62/76, plus newly-spotted 95)
- Zero-ignore baseline approval (down to 1 maintenance entry)

Severity legend:
- **H** — hard block.  Release cannot ship.
- **M** — block unless the exact scenario is documented and the
  release notes call it out as a known issue.

### WASM endpoint — our primary deliverable must work

The browser WASM bundle (`doc/pkg/loft_bg.wasm` + `doc/pkg/loft.js`)
is the primary way users encounter loft — the gallery, the playground,
Brick Buster, and `loft --html` all depend on it.  A release where the
WASM path is broken is a release that doesn't work for most users.

| ID | H/M | Summary | Reference |
|---|---|---|---|
| **WASM-build gate** | H | `cargo build --release --lib --target wasm32-unknown-unknown --no-default-features --features wasm` must succeed with the current stable `rustc`.  The `doc/pkg/` bundle must be rebuilt from this output before tagging. | `Cargo.toml` features, `.github/workflows/ci.yml` |
| **WASM-runtime gate** | H | `tests/html_wasm.rs` must pass: the 5 P137/Q9 tests compile a trivial `.loft` to `--html`, extract the embedded WASM, and run it under Node with stub host imports.  Any `unreachable` trap or instantiation failure blocks. | `tests/html_wasm.rs`, `tools/wasm_repro.mjs` |
| **Gallery smoke** | M | `make gallery` must complete and `doc/gallery.html` must load all 24 examples in a browser without console errors.  Verified by CI (`make test-gl-headless`) where Xvfb is available. | `doc/gallery.html`, `.github/workflows/ci.yml` |

### Crashes — no release may crash on valid input

**No open crash blockers as of 2026-04-15.**  All previously-listed
crash gates closed:

- B2-runtime — closed 2026-04-13 (unit-variant retrofit).
- B3 — closed 2026-04-13 (hidden caller pre-alloc for struct-enum returns).
- B5 — all three layers closed (layers 1+2 2026-04-14; layer 3 closed
  as a side-effect of struct-enum return-slot work in PR #168→#174).
  All four `p54_b5_*` guards green.
- B7 — closed as a side-effect of the B2-runtime / B5 / dep-inference /
  lock-args work across PR #168→#172.  All five `b7_*` guards green
  (the old `_crashes` suffix stays for search-back compatibility).
- P136 — closed (`gen_if` divergent-true-branch fix).
  `tests/wrap.rs::sigsegv_repro_79_alone` and `loft_suite` (which
  walks `79-null-early-exit.loft`) both green; `ignored_scripts()`
  is empty.

### Memory safety — no release may corrupt memory

| ID | H/M | Summary | Reference |
|---|---|---|---|
| **Valgrind-clean gate** | H | `valgrind target/release/loft <script>` must produce `ERROR SUMMARY: 0 errors from 0 contexts` AND `definitely lost: 0 bytes in 0 blocks` on every script in `tests/scripts/` and every doc in `tests/docs/`.  Run on the tag candidate before release. | ROADMAP.md |

### Memory leaks — no release may leak on valid programs

Long-running programs — servers, game loops, REPLs — cannot
tolerate per-iteration leaks.  A release that leaks even one
store per loop iteration is unusable for production workloads;
users hit out-of-memory before the language gets a chance to
prove itself.  This bar isn't a 1.0 feature — it's the floor for
every release.

| ID | H/M | Summary | Reference |
|---|---|---|---|
| **Zero-leak gate** | H | `State::check_store_leaks` must emit no `Warning: N stores not freed at program exit` lines across the full test suite AND a hands-on run of every `tests/scripts/*.loft`.  Today the wrap suite still reports leaks on at least `76-struct-vector-return.loft`, `42-file-result.loft`, and `62-index-range-queries.loft` (1 store each).  Each leak path must be traced and closed — not silenced via `is_locked()` or the `const_refs` skip. | `src/state/mod.rs:1486` check_store_leaks |
| **P122** | H | Store leak in game loops — struct/vector temps not freed at end-of-iteration.  Originally scoped as a Brick Buster ergonomics fix; **generalises** to any loop-body struct/vector construction.  Status-unknown (previously listed as "appears fixed"); must be re-verified in the zero-leak gate above. | PROBLEMS.md |
| **Parallel leak audit** | M | `parallel { ... }` blocks — the A15 structured-concurrency path spawns workers that hold `ParallelCtx`; confirm no worker Stores remain after join.  Run the zero-leak gate with `LOFT_LOG=stores` on `tests/scripts/22-threading.loft`, `80-parallel-block.loft`. | THREADING.md |

### Test suite integrity — no release may silently skip tests

An ignored test is a bug you promised you would fix, then pulled
out of CI.  Every `#[ignore]` hides a known failure — if the
suite is silently skipping them, the release's "all green"
status is a lie.  The bar is simple: **no `#[ignore]` attribute
ships unless explicitly approved with a documented rationale
and a linked issue**.

| ID | H/M | Summary | Reference |
|---|---|---|---|
| **Zero-ignore gate** | H | Every `#[ignore]` (and every `#[ignore = "..."]`) must either be (a) removed because the underlying bug is fixed, or (b) explicitly approved by the release owner with a one-line rationale in `tests/ignored_tests.baseline`.  The approval must cite the blocking issue ID (e.g. `B7 family — ...`, `CI harness SIGABRT (P136-adjacent)`) so the ignore traces back to the open bug.  Unreviewed ignores — where the reason is vague or the owner didn't sign off — block the release. | `tests/ignored_tests.baseline` + `tests/doc_hygiene.rs::ignored_tests_baseline_is_current` |
| **Skip-list audit** | H | Every `SKIP` / `NATIVE_SKIP` / `SCRIPTS_NATIVE_SKIP` / `ignored_scripts()` entry must be traceable to a specific open blocker issue.  "Currently worked around by skipping" counts as an ignore and must appear in the same baseline approval flow. | `tests/native.rs`, `tests/wrap.rs::ignored_scripts`, `tests/native_loader.rs` |

Baseline as of 2026-04-15 — only one entry remains:
- `regen_fill_rs` → maintenance-only, not a test of runtime
  behaviour (regenerates `src/fill.rs`); candidate for
  explicit permanent exemption.

(B5/B7 ignores all removed once the underlying bugs were
confirmed closed; `file_content_nonexistent_trace` and
`sigsegv_repro_79_alone` no longer carry `#[ignore]` attrs.)

Plus `tests/wrap.rs::sigsegv_repro_79_alone` (standalone
`#[ignore]`) and `tests/wrap.rs::ignored_scripts()` skipping
`79-null-early-exit.loft` in `loft_suite` — both tracked under
P136.

---

## Milestone-specific blockers

The items below gate a SPECIFIC milestone (0.9.0 or 1.0.0) without
blocking earlier patch releases that don't claim to ship them.

### Language-surface gaps (0.9.0 blockers)

| ID | H/M | Summary | Reference |
|---|---|---|---|
| **L1** | H | Error recovery — cascading errors after one bad token; high UX impact. | PLANNING.md § L1 |
| **P2** | H | REPL / interactive mode — needed for the "write real loft" story once the browser IDE is deferred past 1.0. | PLANNING.md § P2 |
| **W-warn** | M | Clippy-inspired developer warnings in the interpreter. | PLANNING.md § W-warn |
| **C52** | M | stdlib name clash + `std::` prefix hygiene. | PLANNING.md § C52 |
| **P117** | M | Re-verify the original `file()` pattern with `LOFT_STORES=warn` — fix landed but not re-run end-to-end. | PROBLEMS.md |
| **P120** | M | Full GL example suite end-to-end on a display (fix appears verified; one hands-on pass needed). | PROBLEMS.md |
| **P121** | M | Debug-build valgrind pass over `tests/scripts/50-tuples.loft`. | PROBLEMS.md |
| **P124** | M | `--native-emit` inspection of generated Rust (fix appears verified; one hands-on pass needed). | PROBLEMS.md |

### Stability gate (1.0 blocker)

Safety (valgrind-clean, zero-leak, zero-crash) is tracked under the
[Safety gate](#safety-gate--blocks-every-release) above and is a
blocker for every release, not just 1.0.  The items below are the
1.0-specific additions on top of that floor.

| ID | H/M | Summary | Reference |
|---|---|---|---|
| **Multi-platform binaries** | H | Pre-built binaries published for Linux x86_64-musl, macOS x86_64, macOS aarch64, Windows x86_64-msvc.  Hands-on smoke test of each before publishing the tag. | ROADMAP.md § 1.0.0 |
| **Zero open High issues** | H | No entry in PROBLEMS.md or QUALITY.md tagged **High** severity at release time. | PROBLEMS.md |
| **INCONSISTENCIES sweep** | M | 6 open entries in INCONSISTENCIES.md — none are code blockers but #6 (plain enums cannot have methods) and #10 (sizeof(u8) = 4) need documentation coverage before 1.0. | INCONSISTENCIES.md |

### Code-debt cleanup (nice-to-have for 1.0)

| ID | Summary |
|---|---|
| **P54-U phase 3** | Delete ~540 lines of legacy `src/database/structures.rs::parsing` scanner once a walker-native `Diagnostic` shape replaces the `"line N:M path:X"` error-path format.  Walker already covers the success path (zero fallback hits across the full test suite).  See QUALITY.md § P54-U. |
| **T2-0** | `loft --format` code formatter — professional tooling polish; zero correctness risk. |
| **T1-2** | Wildcard imports (`use mylib::*`) — friction removal; medium payoff. |
| **T1-4** | Match expressions — largest language feature gap.  If deferred past 1.0, INCONSISTENCY #6 must be prominently documented in CHANGELOG.md and the HTML reference. |

Completed historical gate items (T0-1 through T0-7, T1-5, PROBLEMS #10,
#37–#40, P117/P120–P131 fixes, A4 pre-gate, Cargo.toml, README, CHANGELOG,
CI pipeline, R1) are recorded in CHANGELOG.md.

---

## Explicitly out of scope here

The following have their own lifecycles and are **not** tracked as
release blockers in this file.  They may ship before, during, or
after any of the language milestones above — independently:

- **Brick Buster** demo (G3/G5/G6 audio-graphics, BK.*, G7.P itch.io).
- **Moros hex RPG editor** demo (MO.*).
- **Web IDE** shell and multi-file support (W1.1 HTML export kept
  here because it is a language-side feature; W2–W6 are IDE work
  and deferred).
- **Server library** (SRV.*), **game-client library** (GC.*), and
  **scene scripting** layer (SC.*) — these are applications/libraries
  built on top of the language, not part of the language surface.

See [PLANNING.md](PLANNING.md) / [ROADMAP.md](ROADMAP.md) for the
backlogs of those projects.

---

## Explicitly 1.1+ language work

Deferred past 1.0 by design — they are either additive (can land in
a minor) or too large a change to block the stability contract on.

| Item | Notes |
|---|---|
| A2 logger production mode | Low user impact until logger is widely used |
| A4 spacial<T> full implementation | After pre-gate added in 0.8.0 |
| A5 closure capture | Very high effort; depends on P1 |
| C57 route decorator syntax | `@get` / `@post` / `@ws` annotations |
| W1.14 WASM Tier 2 | Web Worker pool + `par()` parallelism |

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
