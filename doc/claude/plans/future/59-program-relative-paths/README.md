<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 59 — Program-relative paths (source-relative asset loading)

## Status

Open — **design decided, cost estimated, ready to phase.** Promoted from issue
[#255](https://github.com/jjstwerff/loft/issues/255) (an enhancement that grew
phase-worthy). The anchor decision is ratified: relative file paths resolve
**program-relative by default**, with a one-line **cwd opt-in** for CLI tools. A
*current* consumer — the **crawler agent** — depends on Phase 1 (it runs generated
programs from a sandbox cwd and needs program-relative asset resolution), so this is
prioritised, not deferred. Effort MH; not yet started.

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

Per [`ISSUE_TRACKING.md` § Designing](../../../ISSUE_TRACKING.md): a design is settled
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
| **0** | `source_dir` correctness — survive `Stores::clone`, populate it in the test runner (prototyped, uncommitted) | S | Open |
| **1** | **Resolver + anchor** — one `resolve_path` chokepoint the ~10 raw `std::fs` file-op sites route through; anchor = source-dir (interp) / exe-dir (native); default program-relative. **Unblocks the crawler.** | M | Open |
| **2** | **cwd opt-in** — the one-line per-program declaration + the runtime flag the resolver checks | S–M | Open |
| **3** | **Corpus migration** — flip the default; the file guards surface the ~27 cwd-dependent files (152 call sites, but per-file opt-in); add the opt-in per file; suite green both backends | S–M (risk) | Open |
| **4** | **Graphics consumer** — `gl_load_font` et al. land on the new anchor; canonical change in external `loft-libs-graphics` (the in-repo fixture is a pinned mirror) | S + cross-repo | Open |

## Phase ordering

`0` and `1` first — **1 unblocks the crawler; ship it before the rest** (`0` is
independent, can land immediately). `2` before `3` (the opt-in must exist before
migrating cwd-dependent files onto it). `4` last (consumes the anchor, cross-repo).

## Open design questions

1. **The cwd opt-in mechanism** — a source directive (e.g. `#cwd` at file top), a
   `loft.toml` flag, or an explicit `cwd(path)` helper. A whole-program directive is the
   friction-free shape ([Goal F](../../../GOALS.md)); decide in Phase 2.
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
- [`ISSUE_TRACKING.md` § Designing](../../../ISSUE_TRACKING.md) — the use-case-matrix
  procedure this plan exercises.
- [`GOALS.md`](../../../GOALS.md) — Goal E (source is the truth) / Goal F (friction-free):
  the anchor + opt-in shape.
