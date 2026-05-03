# 0.8.5 Release — Tooling Checklist

**Release theme:** "loft is learnable" — a first-time visitor installs
loft, gets syntax highlighting in their editor, works through a
30-minute tutorial, and can step through a `--native` build under
stock GDB or LLDB.

**Branch:** `tooling-0.8.5` (off main as of `e6b27a5`).

**Last updated:** 2026-05-03.

---

## Status legend

- ✅ **DONE** — code committed on the branch, ready for quality gate.
- 🚧 **IN-FLIGHT** — partial work committed; next session continues.
- ⬜ **OPEN** — not started; design exists.
- 🟥 **BLOCKED** — waiting on a prerequisite.

---

## Tooling prerequisites

Each tool is **only needed for the specific quality gate noted**.
You can skip the install if you're not running that gate yourself.

### VS Code (for SH.1 visual check + SH.2 install / package check)

VS Code is Microsoft's free editor; the SH.2 extension targets it.
Other editors (Sublime, Vim, Emacs) consume the same TextMate
grammar via different paths — see the SH.1 cross-editor note below
if you want to skip VS Code entirely.

**Install:**
- **Linux:** `sudo snap install code --classic` OR
  download the `.deb` / `.rpm` from <https://code.visualstudio.com/Download>.
- **macOS:** `brew install --cask visual-studio-code` OR
  download the `.dmg`.
- **Windows:** download the installer from <https://code.visualstudio.com/Download>.
- **CLI inside VS Code:** after install, the `code` command should
  be on your PATH.  If not, in VS Code: `Cmd/Ctrl+Shift+P` → "Shell
  Command: Install 'code' command in PATH".

**Verify:**
```sh
code --version    # prints version + commit hash
```

### `vsce` — VS Code extension packager (for SH.2 `vsce package` check)

`vsce` is the official command-line tool that bundles an
`editors/vscode/` directory into a single `.vsix` file you can
install or publish.  It's a Node.js package.

**Install:**
```sh
# requires Node.js (20.x or newer); install Node first if needed:
#   Linux:   sudo apt install nodejs npm   OR   nvm install 20
#   macOS:   brew install node
#   Windows: download from https://nodejs.org/

npm install -g @vscode/vsce
```

**Verify:**
```sh
vsce --version    # prints version
```

**Use (for SH.2 quality gate):**
```sh
cd editors/vscode
vsce package      # produces loft-0.1.0.vsix in the same directory
```

**Reference:** <https://code.visualstudio.com/api/working-with-extensions/publishing-extension>.

### VS Code UI walkthrough — installing + verifying the loft extension

These are the actual menus / panels you'll click through to test
the extension.  Names are stable across recent VS Code versions
(1.75+).

**1. Open the Extensions panel.**

- **Sidebar icon:** four-blocks icon, usually fifth from top.
- **Or menu:** `View > Extensions`.
- **Or keyboard:** `Ctrl+Shift+X` (Windows / Linux) or `Cmd+Shift+X` (macOS).

A panel slides in from the left listing installed + recommended
extensions plus a search box at the top.

**2. Install the local `.vsix`.**

- Click the **`...` (three-dot) menu** in the top-right of the
  Extensions panel header.  This is the "more actions" menu —
  not the gear icon further down.
- Choose **`Install from VSIX...`** from the dropdown.
- A file-picker opens.  Navigate to `editors/vscode/loft-0.1.0.vsix`
  and click **Open**.
- VS Code installs the extension; a notification appears in the
  bottom-right ("loft extension was installed").  No restart
  needed for grammar / snippets.

**Or via CLI** (faster if you're already in a terminal):
```sh
code --install-extension editors/vscode/loft-0.1.0.vsix
```

**3. Verify the extension is loaded.**

- Back in the Extensions panel, type `loft` in the search box.
  The "Loft Language" extension should appear under the
  "Installed" group with a blue/green checkmark.
- Click the entry to see its details panel: README rendered,
  list of contributed languages / snippets / grammars.
- The status bar at the bottom-right of any open `.loft` file
  should show `Loft` as the language mode.

**4. Open a `.loft` file to see highlighting.**

- `File > Open File...` (or `Ctrl/Cmd+O`).
- Pick `examples/hello.loft` (or any file in `tests/docs/`).
- Keywords (`fn`, `for`), types (`integer`, `Point`), strings
  with `{interpolation}`, and comments should colour
  distinctly per your active theme.
- If colours look wrong, the bottom-right language indicator
  may say `Plain Text` instead of `Loft` — click it and pick
  `Loft` from the dropdown.

**5. Try a snippet.**

- In an open `.loft` file, type `fn` and press `Tab`.  Should
  expand to a function template with placeholder fields you
  can `Tab` between.  Repeat with `for`, `match`, `struct`.

**6. Open the Command Palette to confirm tasks work.**

- `Ctrl/Cmd+Shift+P` opens the Command Palette.
- Type `Tasks: Run Task` — if you've added the `tasks.json`
  snippet from `editors/vscode/README.md` to your workspace's
  `.vscode/tasks.json`, "Run loft" should appear.
- Or just press `Ctrl/Cmd+Shift+B` to run the default build
  task (which is "Run loft" once you set it up).

**7. Uninstall (after testing) to keep your profile clean.**

- Extensions panel → search `loft` → click the entry → click
  the **gear icon** in the details header → **Uninstall**.
- Or CLI: `code --uninstall-extension loft-lang.loft`.

**Tip — clean profile for testing:**

If you want to test the extension as a brand-new user would
see it, without your existing settings + theme + extensions
interfering:

```sh
mkdir -p /tmp/loft-clean-vscode
code --user-data-dir=/tmp/loft-clean-vscode --install-extension editors/vscode/loft-0.1.0.vsix
code --user-data-dir=/tmp/loft-clean-vscode examples/hello.loft
```

This launches VS Code with an empty profile that won't interfere
with or be polluted by your normal setup.  Delete `/tmp/loft-clean-vscode`
when you're done.

### Sublime Text (alternative SH.1 cross-editor check)

Skip if you don't have Sublime.  Other TextMate-aware editors
(IntelliJ TextMate import, Atom, BBEdit) work analogously.

**Install:** <https://www.sublimetext.com/download>.

**UI walkthrough:**

1. Open Sublime Text.
2. **`Tools` menu** (top menu bar) → **`Developer`** submenu →
   **`New Syntax...`**.  A new editor tab opens with a YAML
   template.
3. **Replace the entire template** with the contents of
   `syntaxes/loft.tmLanguage.json` (Sublime accepts both YAML
   and JSON `.tmLanguage` formats).
4. **Save** with `Ctrl/Cmd+S`.  Sublime opens its file picker
   in `~/.config/sublime-text/Packages/User/` (Linux) or
   `~/Library/Application Support/Sublime Text/Packages/User/`
   (macOS) by default.  Save as `Loft.sublime-syntax` (or any
   filename — the contents register the syntax).
5. **Open any `.loft` file** (`File > Open...` or
   `Ctrl/Cmd+O`).  Sublime detects the `.loft` extension and
   applies the new syntax automatically.  If colouring is
   absent, click the bottom-right language indicator and pick
   `Loft` from the list.

### IntelliJ-family IDEs (PyCharm, IDEA, WebStorm, CLion, …)

JetBrains IDEs accept TextMate bundles natively — useful if
that's your daily driver.

**UI walkthrough:**

1. **`Settings`** (Linux/Windows) or **`Preferences`** (macOS)
   from the IDE menu.  Or `Ctrl/Cmd+,`.
2. Navigate **Editor → TextMate Bundles**.
3. Click the **`+` button** above the bundle list.
4. Point the file picker at the **`syntaxes/`** directory of
   the loft repo (the directory containing `loft.tmLanguage.json`,
   not the file itself).  IntelliJ auto-detects the grammar.
5. Click **OK** / **Apply**.
6. Open any `.loft` file — colouring should apply.  If not,
   right-click the file in the project tree → `Override File
   Type → Loft`.

### `gdb` — debugger (for NDB.0 quality gate, Linux primary)

GNU Debugger.  Used to verify that `--native-debug` produces a
binary you can step through.

**Install:**
- **Linux:** `sudo apt install gdb` (Debian/Ubuntu) or
  `sudo dnf install gdb` (Fedora) — usually pre-installed.
- **macOS:** `gdb` is awkward on macOS post-Mojave; use **lldb**
  instead (pre-installed with Xcode Command Line Tools).
- **Windows:** comes with `MSYS2` / `mingw-w64` builds; or use
  the Visual Studio Debugger via VS Code's native debugger
  extension.  Easiest path on Windows is WSL2 + Linux gdb.

**Verify:**
```sh
gdb --version    # GNU gdb (...) X.Y
```

**Use (for NDB.0 quality gate):**
```sh
loft --native --native-debug examples/hello.loft
# Note the binary path printed in cache, e.g.
#   examples/.loft/cache/hello-<hash>
gdb examples/.loft/cache/hello-<hash>
(gdb) break n_main
(gdb) run
(gdb) step
(gdb) print var_x
(gdb) continue
(gdb) quit
```

### `lldb` — debugger (NDB.0 alternative, macOS primary)

LLVM Debugger.  Same use case as GDB; the canonical macOS choice.

**Install:**
- **macOS:** comes with Xcode Command Line Tools (`xcode-select --install`).
- **Linux:** `sudo apt install lldb` or `sudo dnf install lldb`.
- **Windows:** ships with LLVM (<https://releases.llvm.org/>).

**Verify:**
```sh
lldb --version
```

**Use (for NDB.0 quality gate):**
```sh
lldb examples/.loft/cache/hello-<hash>
(lldb) breakpoint set --name n_main
(lldb) run
(lldb) step
(lldb) frame variable
(lldb) continue
(lldb) quit
```

### `objdump` — DWARF inspector (NDB.0 sanity check)

Standard binutils tool.  Confirms DWARF debug-info actually made
it into the binary, independent of whether GDB / LLDB cooperate.

**Install:**
- **Linux:** `sudo apt install binutils` — usually pre-installed.
- **macOS:** `brew install binutils` (provides `gobjdump`; the
  Apple-shipped `objdump` is LLVM-based and uses different flag
  names).
- **Windows:** ships with MSYS2 / mingw-w64 / WSL2.

**Use (for NDB.0 quality gate):**
```sh
objdump --dwarf=info examples/.loft/cache/hello-<hash> | head -20
# Should show DW_TAG_subprogram + DW_AT_name entries.
```

### `node` — for JS-glue probes (browser quality gate)

Already present if you installed Node for `vsce`.  The
`make gallery` step uses Node to instantiate the WASM bundle as
a browser would, catching `LinkError` mismatches before they
reach users.

**Use (already wired into CI):**
```sh
make gallery   # rebuilds doc/pkg/ + probes the bundle
```

### `python3` — JSON validation (universal, already common)

Used for one-line JSON schema validation:
```sh
python3 -c "import json; json.load(open('syntaxes/loft.tmLanguage.json'))"
```

If `python3` exits 0, the JSON parses; if not, you get a clear
`JSONDecodeError` line + col.  Already part of every modern OS.

### Quick "what do I need for which gate?" table

| Gate | Tool needed | Skip if… |
|---|---|---|
| SH.1 visual (VS Code) | VS Code | you have Sublime / Vim / Emacs and use those instead |
| SH.1 visual (Sublime) | Sublime Text | you have VS Code |
| SH.2 `vsce package` | `vsce` (+ Node) | you'll publish from CI later |
| SH.2 install | `code` CLI | you'll smoke-test in your daily editor |
| NDB.0 GDB | `gdb` | you're on macOS — use LLDB |
| NDB.0 LLDB | `lldb` | you'll run on Linux only |
| NDB.0 DWARF | `objdump` | you trust GDB / LLDB to find the source |
| Browser probe | `node` | not interested in the gallery / playground deploy |
| Cross-cutting CI | none extra | (CI matrix on GitHub does this) |

---

## Per-item checklist

### SH.1 — TextMate grammar (`syntaxes/loft.tmLanguage.json`) — ✅ DONE

**Effort:** S (~1 hour) — actual: ~45 min.
**Source design:** `doc/claude/DX.md` § SH.1.
**Commit:** _(this branch)_.

#### Build checklist

- [x] `syntaxes/loft.tmLanguage.json` created with the full DX.md SH.1
      scope-mapping table.
- [x] String interpolation `{expr}` as a nested scope.
- [x] `{{` `}}` literal-brace escapes.
- [x] Hex / binary / octal / decimal / float number variants.
- [x] Doc comments (`///`) distinct from line comments (`//`).
- [x] Test annotations (`@EXPECT_*`) distinct from compiler
      annotations (`#rust`, `#native`).
- [x] `syntaxes/README.md` documenting scope coverage inline.
- [x] JSON validity verified via `python3 -m json.tool`.

#### Quality gates (before ship)

- [ ] **Visual sanity check** — open `tests/docs/00-general.loft`,
      `tests/docs/06-function.loft`, `tests/docs/09-enum.loft` in VS
      Code with the SH.2 extension installed.  Confirm:
  - keywords (`fn`, `for`, `if`, `match`, `return`) are highlighted
  - type names (`integer`, `text`, `Position`, `CamelCase`) are
    distinct from identifiers
  - strings with `{interpolation}` don't break out of the string scope
  - `///` doc comments are visually distinct from `//` line comments
  - `#rust"..."` annotations colour the `#rust` part as annotation,
    the `"..."` part as a string
  - `@EXPECT_ERROR` test annotations are visible (any reasonable
    colour theme should style them)
- [ ] **Negative check** — open a `.loft` file with deliberate syntax
      errors (unclosed string, missing brace).  Highlighting should
      degrade gracefully — no infinite-loop spinner, no all-red.
- [ ] **Cross-editor check** — load the grammar in Sublime Text via
      `Tools > Developer > New Syntax`; same colouring should apply.
- [ ] **GitHub Linguist check** (later, requires fork + PR) — file at
      `vendor/grammars/loft/syntaxes/loft.tmLanguage.json` works for
      `.loft` extension.  **Skip for 0.8.5; track as 0.8.6 follow-up.**

#### Risks

- TextMate grammars are notoriously difficult to test programmatically.
  Visual inspection in VS Code is the canonical acceptance check.
- String-interpolation nesting can break if the grammar's
  `meta.interpolation` doesn't terminate cleanly on `}`.  The grammar
  uses `begin: "\\{"` / `end: "\\}"` which is the documented pattern;
  manual verification on `tests/docs/02-text.loft` (heavy
  interpolation) is the test.

---

### SH.2 — VS Code extension scaffold (`editors/vscode/`) — ✅ DONE

**Effort:** S (~1 hour) — actual: ~30 min.
**Source design:** `doc/claude/DX.md` § SH.2.
**Commit:** _(this branch)_.

#### Build checklist

- [x] `editors/vscode/package.json` — extension manifest declaring
      `.loft` language id + scope name + grammar path + snippet path.
- [x] `editors/vscode/language-configuration.json` — comment toggle,
      bracket matching, auto-close, indent rules.
- [x] `editors/vscode/snippets/loft.json` — 12 snippets: `fn`, `fnv`,
      `main`, `struct`, `enum`, `for`, `while`, `match`, `if`, `ife`,
      `assert`, `println`.
- [x] `editors/vscode/syntaxes/loft.tmLanguage.json` — symlink to
      `../../../syntaxes/loft.tmLanguage.json` (single source of truth).
- [x] `editors/vscode/README.md` with install instructions, feature
      list, run-current-file `tasks.json` snippet.

#### Quality gates (before ship)

- [ ] **`vsce package` succeeds** without errors or warnings.
      Produces `loft-0.1.0.vsix`.
- [ ] **`code --install-extension loft-0.1.0.vsix` succeeds** on a
      clean VS Code profile.
- [ ] **Open `tests/docs/00-general.loft`** — confirm highlighting,
      bracket matching, comment toggling (`Ctrl+/`), and at least one
      snippet (`fn` + Tab) all work.
- [ ] **Verify the symlink survives `vsce package`** — extract the
      packaged `.vsix` (it's a zip), confirm `syntaxes/loft.tmLanguage.json`
      is a real file inside (not a broken symlink).
- [ ] **Marketplace publish** — DEFERRED.  Requires a Personal Access
      Token from `dev.azure.com` and the publisher account
      `loft-lang`.  Track as a separate ship task; users can install
      from `.vsix` in the meantime.

#### Risks

- `vsce` may complain about missing `LICENSE.md` or `repository.url`
  without a sample `README.md` in the extension dir.  The supplied
  README + manifest should satisfy the basic checks; verify on first
  `vsce package` run.
- Symlinks survive most filesystem-level packaging tools but some
  Windows-specific edge cases exist.  Test on macOS / Linux first;
  Windows verification is a separate quality gate.

---

### DX.1 — Quick-start `examples/` directory — ⬜ OPEN

**Effort:** XS (~1-2 hours).
**Source design:** `doc/claude/DX.md` § DX.1.
**Commit:** _(not yet)_.

#### Build checklist (per design)

- [ ] `examples/hello.loft` — Hello world; `println` + string
      interpolation.
- [ ] `examples/fibonacci.loft` — Recursive + iterative; functions,
      loops, recursion.
- [ ] `examples/fizzbuzz.loft` — If/else, modulo, format strings.
- [ ] `examples/structs.loft` — Point + distance; structs, methods,
      math.
- [ ] `examples/collections.loft` — Vector + sorted + hash; collection
      types, iteration.
- [ ] `examples/match.loft` — Pattern matching on enums.
- [ ] `examples/files.loft` — Read/write a text file; file I/O.

#### Per-file requirements (per design)

- [ ] Each file runnable standalone: `loft examples/<name>.loft`.
- [ ] No dependencies on `lib/` packages.
- [ ] Each file < 30 lines.
- [ ] Comments explaining the key concept the file demonstrates.
- [ ] Output self-explanatory — no "test passed", show meaningful
      results.

#### Quality gates (before ship)

- [ ] **All seven examples run cleanly** — `for f in examples/*.loft;
      do loft "$f" || exit 1; done` returns 0.
- [ ] **Same under `--native`** — every example also passes
      `loft --native examples/<name>.loft`.
- [ ] **Output review** — read each example's output as a first-time
      user.  Is the meaningful result obvious?  If `fibonacci.loft`
      prints `1 1 2 3 5 8 13 21 34 55`, that's good.  If it prints
      `result=55`, less good (no context).
- [ ] **README.md update** — append the "Examples" section per the
      design's snippet.
- [ ] **Add to CI** — `tests/scripts/` already runs everything in
      `tests/scripts/`; add `examples/` to that suite OR a new
      `examples` test that mirrors `native_scripts` shape.  Catches
      regressions where a stdlib change breaks an example.

#### Risks

- `files.loft` requires file I/O — the example must be hermetic
  (write to `/tmp/` or a temp file the example creates + cleans up
  itself).  Otherwise running it twice from different cwds may fail.
- `collections.loft` may be longer than 30 lines if it covers vector
  + sorted + hash all in one.  Either trim to one collection type per
  example or accept the line-count bump.

---

### DX.3 — "Learn loft in 30 minutes" walkthrough — ⬜ OPEN

**Effort:** S (~half a day).
**Source design:** _(no doc — design inline below; the existing
`doc/claude/DX.md` § DX.3 is stale and refers to the old error-message
item that plan-07 phase 2 already shipped)_.
**Commit:** _(not yet)_.

#### Suggested structure (10 sections, ~3 minutes each)

| § | Title | Content |
|---|---|---|
| 1 | Hello, Loft | Install loft, run `loft examples/hello.loft`, edit it |
| 2 | Variables and types | No `let`; type inference; `assert` |
| 3 | Functions | `fn`, params, return type, default args, last-expr return |
| 4 | Control flow | `if`, `else`, `for`, `while`, `match` (one liners) |
| 5 | Strings | `"text"`, `{interpolation}`, `{{` escape, `len`, `+` |
| 6 | Collections | `vector`, `sorted`, `hash` — one example each |
| 7 | Structs | Define, construct, method-style call (`p.distance(q)`) |
| 8 | Enums | Plain enum + struct-enum variant; `match` on it |
| 9 | Files and JSON | Read a file, parse JSON, print a field |
| 10 | Where next | Pointer to STDLIB.md / DX.md / lib/ + GitHub issue tracker |

#### Build checklist

- [ ] `doc/learn-loft.md` (or similar — pick a stable URL slug since
      we'll link to it from the README + extension marketplace).
- [ ] Each section ≤ 200 words of prose + 1-2 runnable code blocks.
- [ ] Every code block extracted from `examples/` (DX.1) so it's
      pre-verified to run.
- [ ] Closing section links to STDLIB.md, the examples directory,
      the GitHub issue tracker for questions.
- [ ] README.md links to the walkthrough in the install / first-run
      section.

#### Quality gates (before ship)

- [ ] **Time check** — read it cold, top-to-bottom, with no prior
      loft exposure.  Should take 25-35 minutes including running
      each code block.  If > 35, trim a section.
- [ ] **External-reader check** — per ROADMAP ship criterion: "One
      external programmer (outside the loft project) can install
      SH.2 from VS Code Marketplace, open `examples/10-2d-canvas.loft`,
      read DX.3 top-to-bottom, and run the demo within 30 minutes
      from zero prior exposure.  Hands-on feedback collected before
      tagging."
- [ ] **No broken examples** — every code block in the walkthrough
      runs as-is (CI extracts and runs them; or they're in
      `examples/` already and the walkthrough just embeds them).
- [ ] **Linker discipline** — every term that has a STDLIB.md /
      LOFT.md entry links to it on first use.  No orphan terminology.

#### Risks

- "30 minutes" is a hand-wavy goal.  Acceptance = real external
  reader hits it.  Without that, the walkthrough is unverified
  marketing copy.
- The example `10-2d-canvas.loft` mentioned in the ROADMAP ship
  criterion doesn't exist in `examples/` per DX.1's table.  Either
  add it to DX.1's list or update the ship criterion to pick an
  existing example (`structs.loft` would work as a graphical-ish
  hook).

---

### NDB.0 — `--native-debug` flag (DWARF in `--native` builds) — ⬜ OPEN

**Effort:** XS (~1-2 hours).
**Source design:** `doc/claude/NATIVE_DEBUG.md` § NDB.0.
**Commit:** _(not yet)_.

#### Build checklist (per design)

- [ ] Add `--native-debug` CLI flag in `src/main.rs` (parse + bool flag).
- [ ] When flag is set:
  - [ ] Push `-Cdebuginfo=2 -g` to the rustc invocation.
  - [ ] Drop `-O` (debug builds; opt level 0) UNLESS `--native-release`
        is also set.
  - [ ] Preserve `/tmp/loft_native_<pid>.rs` (per-process tmp) on
        disk so DWARF's `.debug_line` table points at a real file.
        Today's behaviour deletes it after compilation.
  - [ ] Emit a `.loft.map` sidecar next to the binary (NDB.1 consumes
        this; NDB.0 emits it but doesn't use it).  Defer the format
        of `.loft.map` to NDB.1; for NDB.0 either emit a stub `{}`
        or skip emission entirely (decide at impl time).

#### Quality gates (before ship)

- [ ] **Build works** — `loft --native --native-debug examples/hello.loft`
      produces a runnable binary at `/tmp/loft_native_bin_<pid>` (or
      a user-provided path).
- [ ] **Binary contains DWARF** — `objdump --dwarf=info /tmp/loft_native_bin_<pid>
      | head -20` shows non-empty DWARF info.
- [ ] **Source file present** — `/tmp/loft_native_<pid>.rs` exists
      after the build (not deleted).
- [ ] **GDB step works** — per the design's expected interaction:
      ```
      $ gdb /tmp/loft_native_bin_<pid>
      (gdb) break n_main
      (gdb) run
      Breakpoint 1, n_main () at /tmp/loft_native_<pid>.rs:NN
      (gdb) step
      (gdb) print var_x
      ```
      Variable names will be rust-internal (`var_x` not `x`) — that's
      expected for NDB.0; NDB.1 adds the source map.
- [ ] **LLDB step works** — same flow with `lldb` instead of `gdb`.
      Test on macOS if access is available; Linux LLDB also works.
- [ ] **`--native-release` interaction** — `loft --native
      --native-release --native-debug examples/hello.loft` produces an
      optimised binary WITH debug info.  Confirms the two flags
      compose.

#### Risks

- DWARF generation can be slow (10-30s extra for medium programs).
  Document this in the help text so users don't think `loft --help`
  is hanging.
- If the `.rs` file is deleted post-compile (current behaviour),
  GDB will show `(no source)` even with debug info present.  Test
  carefully.

---

## Cross-cutting quality gates

These apply to the release as a whole, not per-item:

### Repository hygiene

- [ ] `cargo fmt --all -- --check` clean.
- [ ] `cargo clippy --tests --release -- -D warnings` clean.
- [ ] `cargo test --release --test issues` — 540/540.
- [ ] `cargo test --release --test threading` — 47/47.
- [ ] `cargo test --release --test threading_chars` — 35/35.
- [ ] `cargo test --release --test codegen_emitter` — 18/18.
- [ ] `cargo test --release --test error_messages` — 2/2.
- [ ] `cargo nextest run --profile ci` (matches CI-locally).
- [ ] CI matrix on push (Windows / macOS / Linux) all green.

### Safety gate (per RELEASE.md)

- [ ] WASM endpoint verified green (`make wasm-html-test`).
- [ ] No new crashes on the test corpus.
- [ ] Memory leak gate: wrap-suite `loft_suite` emits no
      `stores not freed` warnings.
- [ ] PROBLEMS.md has zero new H-priority entries opened during
      this release window.

### Cross-platform smoke test

- [ ] **Linux:** install loft from a fresh git clone, run the
      walkthrough top-to-bottom, install the VS Code extension,
      open an example.
- [ ] **macOS:** same.
- [ ] **Windows:** same — pay attention to symlink behaviour (the
      VS Code extension grammar symlink is the most likely point
      of failure on Windows).

### Release artefacts

- [ ] `Cargo.toml` version bumped to `0.8.5`.
- [ ] `CHANGELOG.md` entry written for users (not contributors).
- [ ] `doc/claude/CHANGELOG_TECHNICAL.md` entry written for
      contributors (per-commit detail).
- [ ] `doc/claude/RELEASE.md` "0.8.5 progress" subsection updated
      to reflect ship.
- [ ] Tag created and pushed (`v0.8.5`).
- [ ] `release.yml` workflow runs and produces artefacts.

---

## Ship-criterion external test (per ROADMAP)

The ROADMAP § 0.8.5 ship criteria state:

> One external programmer (outside the loft project) can install
> SH.2 from VS Code Marketplace, open `examples/10-2d-canvas.loft`,
> read DX.3 top-to-bottom, and run the demo within 30 minutes from
> zero prior exposure.  Hands-on feedback collected before tagging.

Two issues with this as written today:

1. SH.2 is not yet on the marketplace (deferred — install from
   `.vsix` is the v1 path; marketplace publish gates on a separate
   Personal Access Token step).
2. `examples/10-2d-canvas.loft` doesn't exist — DX.1's table doesn't
   include it.  Either add it (the file would demonstrate the 2D
   canvas / OpenGL bindings, an existing capability) or pick a
   different example for the ship criterion.

**Recommendation:** for the 0.8.5 ship criterion, swap to a
small but visually impressive example like `examples/hello.loft`
+ `examples/structs.loft`.  Add `examples/canvas.loft` only if the
2D canvas API surface is stable enough to demo.

---

## Out of scope for 0.8.5 (tracked separately)

- **Marketplace publish of SH.2** — separate publishing step;
  needs Personal Access Token + `vsce publish` invocation.
- **GitHub Linguist contribution** — submit the grammar to the
  Linguist repo; gets `.loft` colour highlighting in GitHub UI
  for free.  0.8.6 follow-up.
- **NDB.1** — source-map-aware GDB / LLDB plugins.  0.8.6 work.
- **LSP server** — a real LSP daemon exposing diagnostics /
  completion / go-to-definition.  0.8.6 LSP.1.

---

## Decision points

1. **Should DX.1 add a `canvas.loft` example?**  Optional but it's
   what the ROADMAP ship criterion references.  If yes: ~1 hour
   extra.  If no: edit the ship criterion to pick a different
   example.

2. **Does the walkthrough live at `doc/learn-loft.md` or somewhere
   else?**  The README will link to whatever path we pick; pick now
   so the README + extension marketplace point at the same URL.

3. **NDB.0 — emit `.loft.map` stub or skip?**  Stub is cleaner for
   NDB.1; skip is less code today.  Pick at impl time.

4. **External-reader test — who?**  The ROADMAP ship criterion is
   binding.  Need at least one external programmer to do the
   walkthrough cold.  Identify before tagging.

---

## Quality-evaluation summary

| Item | Status | Quality gates passed | Quality gates open |
|---|---|---|---|
| SH.1 | ✅ DONE | 0/4 | Visual + cross-editor + Linguist |
| SH.2 | ✅ DONE | 0/5 | vsce package + install + open + symlink + marketplace |
| DX.1 | ⬜ OPEN | n/a | All (after build) |
| DX.3 | ⬜ OPEN | n/a | All (after build) |
| NDB.0 | ⬜ OPEN | n/a | All (after build) |
| Cross-cutting | n/a | n/a | All |
| External test | n/a | n/a | Identify external reader |

**Total quality gates open:** ~30 across all items.

**Pre-ship critical path:**
1. Finish DX.1 (examples) — XS.
2. Finish NDB.0 (native-debug flag) — XS.
3. Write DX.3 (walkthrough) — S; depends on DX.1.
4. Run all visual / vsce / GDB quality gates — manual but
   bounded.
5. Run cross-cutting CI gate.
6. Identify external reader, run them through.
7. Tag.

**Estimated remaining effort:** 6-10 hours of code/docs work +
1-3 hours of manual testing + external-reader booking time.
