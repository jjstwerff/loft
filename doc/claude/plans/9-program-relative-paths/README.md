<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 9 — Program-relative paths (source-relative asset loading)  ·  `@PLN9` ([loft-lang/plans#9](https://github.com/loft-lang/plans/issues/9))

## Status

**Shipped on `local_assets` (interp + native) — program-relative by default with the
`#cwd` opt-in.** Tracked as [`@PLN9`](https://github.com/loft-lang/plans/issues/9)
(loft-lang/plans); promoted from loft issue
[#255](https://github.com/jjstwerff/loft/issues/255). A relative file path resolves
against the program's own directory by default; CLI tools opt back into cwd with the
one-line `#cwd` file directive.

Landed (Phases 0 → 3):
- **0** — `source_dir()` populated once at parse time (`Parser::parse`, the single
  home); `Stores::clone` preserves it.
- **1a** — native anchor: `source_dir()` = the executable's dir via `current_exe()`.
- **1b** — the `resolve_path` chokepoint (`Stores::resolve_path`) every file-op site
  routes through (interp io.rs + database/io.rs + png; native codegen_runtime; the
  standalone delete/move/mkdir ops in fill.rs + the `#rust` templates).  Resolution
  happens at the OS boundary, so `File.path` keeps the value the user passed.
- **2** — the `#cwd` file directive opts a program out → cwd.
- **3** — `Stores::new` defaults program-relative (the flip); native bakes the
  parse-time value via `const LOFT_PROGRAM_RELATIVE`; the corpus migration added `#cwd`
  to the 13 tests that do cwd-relative file I/O.  Also a `LOFT_PATHS=program|cwd`
  per-invocation override.

Verified on both backends: default rehomes a bundled asset from a foreign cwd; `#cwd`
and `LOFT_PATHS=cwd` stay cwd; absolute untouched; program-relative write/read/delete
round-trips with no leak.  Interp suite green (wrap 50/0, issues 684/0); native_scripts
+ native_dir green.

Open: **1w** — the wasm anchor *code* is wired (`source_dir()` = the host working dir
via `current_dir()`), but running `191` under wasm is gated on
[#268](https://github.com/jjstwerff/loft/issues/268) (wasip2 `print()` codegen calls an
undeclared `loft_host_print`); and **4** the graphics consumer (`gl_load_font` on the
new anchor, in external `loft-libs-graphics`).

## Goal

A relative file path resolves against **the program's own location** by default (the
source dir under `--interpret`/test, the executable's dir under `--native`), so
"program + assets" is a portable bundle that runs from any cwd; a CLI program opts into
**cwd** resolution in one line.

## Effort + design

- **Effort:** MH — cross-cutting (runtime file-op layer + native + parser + the
  existing corpus + an external lib), behaviour-changing, multi-phase.
- **Design:** ✓ — anchor decided; the use-case matrix below drove it.
- **Last touched:** 2026-06-04

## The design evaluation — the use-case matrix (served *and* broken)

Per [`ISSUE_TRACKING.md` § Designing](../../ISSUE_TRACKING.md): a design is settled
only when **both halves** of its use-case matrix are enumerated. The *broken* half is
what flipped the answer from a naive "switch the anchor" to "single anchor + default +
opt-in."

**Served** (program-relative anchor):
- A program loads its **bundled assets** (font / sprite / data beside the source or
  binary) by a relative path, regardless of cwd → a portable bundle (game distribution,
  the lavition path).
- The **crawler agent** runs its generated programs from a sandbox cwd and addresses
  their assets program-relative. ← the current dependency.

**Broken if we just switch the anchor globally** (the half that did the work):
- **CLI tools** resolving a *user-supplied* relative path (`loft tidy.loft data.csv` —
  `data.csv` is in the *user's* cwd, not beside `tidy.loft`).
- Scripts writing `./output.txt` relative to where they ran.
- Unix composability (`cd` / `find` / `xargs` / pipelines).

**What it reveals:** two *kinds* of relative path — program-bundled (program-relative)
vs user/environment-supplied (cwd) — needing different anchors. So: **one anchor per
program, no fallback** (a cwd→program fallback silently loads the wrong file when a user
file is merely missing — brittle); **default program-relative**; **one-line cwd opt-in**.
The existing file guards make a wrong default *visible* (null + warning), not silent —
which is what makes program-relative safe to default to.

## Composition matrix — Stage A (conformance, per phase, before merge)

Validate per **file-op × backend × anchor-mode × path-kind**:
- backend → anchor: `--interpret` = source dir · `--native` = exe dir · test runner =
  test-file dir;
- mode: default (program-relative) · cwd opt-in;
- path-kind: relative-bundled (re-homes) · relative-user (cwd under opt-in) · absolute
  (MUST be untouched).
A green cell: an `assets/x` path resolves to the program-anchored file, an absolute path
is unaffected, and the opt-in flips resolution to cwd — on the right backend.

## Sub-arcs

| Phase | Item | Effort | Status |
|---|---|---|---|
| **0** | `source_dir` correctness — populate once at parse time (`Parser::parse`, single home); `Stores::clone` preserves it. Regression `191-source-dir.loft`. | S | **Shipped** (`c2979ff3`) |
| **1a** | **Native anchor** — `source_dir()` = exe dir via `current_exe()` under `--native` (was ""). interp + native green. | S | **Shipped** (`f2a7fafe`) |
| **1b** | **Resolver chokepoint** — `Stores::resolve_path`, the single home every file-op site routes through (interp io.rs + database/io.rs + png; native codegen_runtime ×5; standalone delete/move/mkdir in fill.rs + `#rust` templates). Resolves at the OS boundary (keeps `File.path`). | M | **Shipped** (`7519de96`) |
| **2** | **`#cwd` opt-in** — file-level directive parsed in `parse_file`; native bakes it via `const LOFT_PROGRAM_RELATIVE`. + `LOFT_PATHS` env override. | S | **Shipped** (`7519de96`) |
| **3** | **Default flip + corpus migration** — `Stores::new` defaults program-relative; 13 cwd-relative tests migrated with `#cwd`; suite green both backends. | S–M | **Shipped** (`7519de96`) |
| **1w** | **Wasm anchor** — `source_dir()` = host working dir via `current_dir()` (was "" under WASI). Code wired; 191 wasm-run gated on [#268](https://github.com/jjstwerff/loft/issues/268). | S | **Shipped** (`25feaac2`) · test gated on #268 |
| **4** | **Graphics consumer** — `gl_load_font` et al. land on the new anchor; canonical change in external `loft-libs-graphics` (the in-repo fixture is a pinned mirror) | S + cross-repo | Open |

## Phase ordering

`0` and `1` first — **1 unblocks the crawler; ship it before the rest** (`0` is
independent, can land immediately). `2` before `3` (the opt-in must exist before
migrating cwd-dependent files onto it). `4` last (consumes the anchor, cross-repo).

## Open design questions

1. **The cwd opt-in mechanism** — a source directive (e.g. `#cwd` at file top), a
   `loft.toml` flag, or an explicit `cwd(path)` helper. A whole-program directive is the
   friction-free shape ([Goal F](../../GOALS.md)); decide in Phase 2.
2. **Test-runner anchor** — the test *file's* dir, or the package root? (Affects how
   library tests find fixtures.)

## Cross-arc dependencies

- **Crawler agent** — depends on Phase 1 (program-relative resolution for its generated
  programs). The reason this plan is prioritised over deferral.
- **`loft-libs-graphics`** (external) — Phase 4's canonical `gl_load_font` change lands
  there; the in-repo `tests/fixtures/libs/graphics/` is a pinned mirror (never edit as
  the fix — `sync-fixtures.sh` re-homes it).

## See also

- Issue [#255](https://github.com/jjstwerff/loft/issues/255) — the use-case framing, the
  ratified decision, and this cost breakdown (the lightweight capture; this plan is the
  design + sequencing).
- [`ISSUE_TRACKING.md` § Designing](../../ISSUE_TRACKING.md) — the use-case-matrix
  procedure this plan exercises.
- [`GOALS.md`](../../GOALS.md) — Goal E (source is the truth) / Goal F (friction-free):
  the anchor + opt-in shape.
