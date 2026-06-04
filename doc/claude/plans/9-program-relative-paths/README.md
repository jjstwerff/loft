<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 9 — Program-relative paths (source-relative asset loading)  ·  `@PLN9` ([loft-lang/plans#9](https://github.com/loft-lang/plans/issues/9))

## Status

**In progress — the per-backend anchor is shipped; the resolver + default flip is the
next arc.** Tracked as [`@PLN9`](https://github.com/loft-lang/plans/issues/9)
(loft-lang/plans); promoted from loft issue
[#255](https://github.com/jjstwerff/loft/issues/255). The anchor decision is ratified:
relative file paths resolve **program-relative by default**, with a one-line **cwd
opt-in** for CLI tools.

Landed on `local_assets` (Phase 0 + Phase 1a):
- **Phase 0** — `source_dir()` is populated once at parse time (`Parser::parse`, the
  single home), so it works on every interpreter execution path (was empty under the
  wrap / `loft --test` runners). `Stores::clone` preserves it.
- **Phase 1a** — `source_dir()` now has a real **native** anchor (the executable's
  directory via `current_exe()`); interp = source dir, native = exe dir. Regression
  `tests/scripts/191-source-dir.loft` runs on interp + native.

The reframe that set the sequencing: `source_dir()` working IS the crawler's unblock —
its generated code can anchor explicitly (`file("{source_dir()}/x")`). The
`resolve_path` chokepoint + the program-relative **default flip** is the *ergonomic*
layer and the behaviour-changing part, so it is its own deliberate arc (Phases 1b–3).

Open: the resolver chokepoint (18 file-op sites), the cwd opt-in, the corpus migration
(~13 files re-home under the flip — confirmed, e.g. `19-files.loft`), the wasm host
anchor, and the graphics consumer. Effort MH.

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
| **1b** | **Resolver chokepoint** — one `resolve_path` the **18** raw file-op sites route through (interp `io.rs` ×8, native `codegen_runtime.rs` ×5, the `file()` ctor ×3, PNG ×1, +2 wasm bridges); anchor = `source_dir()`. Built as a passthrough first (default cwd, no behaviour change). | M | Open |
| **2** | **cwd opt-in** — the one-line per-program declaration + the runtime flag the resolver checks (open question: directive vs `loft.toml` vs helper) | S–M | Open |
| **3** | **Default flip + corpus migration** — flip default to program-relative; the file guards surface the cwd-dependent files (~13 confirmed, e.g. `19-files.loft`'s `file("tests/example")`); add the opt-in per file; suite green both backends | S–M (risk) | Open |
| **1w** | **Wasm anchor** — host-supplied `source_dir()` (`current_exe()` unreliable under WASI); un-skip `191` for wasm | S | Open |
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
