<\!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Plan-12 — library test self-sufficiency

Part of [@PLAN12 library extraction](README.md).  Covers
**Phase 6t** — Tiers 1-5 of library test coverage gaps.
Each extracted chunk must be testable standalone post-
extraction; this phase tracks the gap-closing work for the
libraries that today rely on monorepo-owned Rust harnesses
(`tests/multiplayer_v{2,3,5}.rs`, `tests/graphics_gold.rs`,
etc.) or have no test infrastructure of their own.

Includes the cross-package native-link blocker (@P389) that
shapes the Tier 3 multiplayer harness ship-site decision.

---

### Phase 6t — library test self-sufficiency

**Problem.**  Audit of test coverage (2026-05-28) found 4 of 11
libraries are not independently testable post-extraction.  The gap is
two clusters of monorepo-owned Rust harnesses that drive library code
but live outside `lib/<name>/tests/`:

| Cluster | Files | Tests | Library subjects | Why Rust-side |
|---|---|---|---|---|
| Graphics gold-image regression | `tests/graphics_gold.rs` + `tests/gold/*.png` | 8 | `lib/graphics` | PNG decode + per-channel MAE tolerance compare against checked-in reference PNGs.  Pure-loft can't replicate the tolerance algorithm; encoder drift means byte-compare is brittle. |
| Multiplayer integration | `tests/multiplayer_v{2,3,5}.rs` | 10 (v2: 3, v3: 2, v5: 5) | `lib/server`, `lib/web`, `lib/game_protocol` | Subprocess orchestration to dodge @P245 (single-process `parallel{}` + I/O hangs when one arm accepts and another connects to a loopback port).  Must run client + server as separate processes. |

Plus two thin loft-side gaps (Tier 1, mechanical):

- `tests/scripts/130-gridmesh-crystal-equiv.loft` — gridmesh C1 SegMesh equivalence vs legacy CrystalMesh.
- `tests/scripts/133-crystal-incr.loft` — incremental crystal update.

Both reference `audience_crystal` (monorepo-paired) so the copy
must substitute a synthetic `CellSnap` fixture inside `lib/gridmesh/tests/`.

**Out of scope — these stay in loft.**  `tests/wrap.rs` and
`tests/native.rs` are the discovery harnesses that enumerate every
`lib/<pkg>/tests/*.loft`; they wire the per-library tests into
`make ci` and are not subject to extraction.  `tests/leak.rs`,
`tests/runtime_warnings.rs`, `tests/codegen_emitter.rs`,
`tests/issues.rs`, `tests/extraction_hygiene.rs` are
compiler/runtime regressions that use library code as a fixture, not
as the subject under test — they belong to the loft toolchain.

**Tier 1 — mechanical copies (XS, no design needed). DONE.**

Outcome (verified 2026-05-29): the two `tests/scripts/13X-...loft`
files were folded into `lib/gridmesh/tests/segmesh.loft` (the
crystal-equivalence + incremental-update assertions live alongside
the segmesh's own tests rather than as separate files).  The
`use audience_crystal;` block was replaced with a synthetic
`CellSnap`-shaped fixture as specified.  The originals are gone
from `tests/scripts/`.  `cd lib/gridmesh && loft test` reports
20 passed across 4 files.

**Tier 2 — port `graphics_gold.rs` to `lib/graphics/native/tests/` (M).**

`lib/graphics` already has a Rust crate at `lib/graphics/native/`
(the cdylib).  Add `lib/graphics/native/tests/gold.rs` as a Rust
integration test inside that crate, carrying:

- The 8 `#[test]` functions (same names, same examples driven from
  `lib/graphics/examples/`).
- The PNG decode + MAE compare helper.
- The reference PNGs — move `tests/gold/*.png` →
  `lib/graphics/tests/gold/*.png` (loft-package convention; Rust
  test reads them from there via a workspace-relative path).
- `UPDATE_GOLD=1` env var behaviour preserved.

When `lib/graphics` extracts to `loft-libs-graphics`, its native
crate + integration test travel together.  The `library-ci.yml`
template needs one new step per library that has a `native/tests/`
directory: `cd lib/<name>/native && cargo test --release`.

*Verify:* `cd lib/graphics/native && cargo test --release` runs the
8 tests; deleting `tests/graphics_gold.rs` from the monorepo leaves
coverage intact.

**Tier 3 — port multiplayer harnesses to `loft-libs-net` (MH).**

The 10 subprocess-orchestrated tests across `multiplayer_v2.rs`
(3 tests), `multiplayer_v3.rs` (2 tests), and `multiplayer_v5.rs`
(5 tests) test the surface that `lib/server` + `lib/web` +
`lib/game_protocol` *jointly* expose — no single library owns
them.  (Inventory verified 2026-05-29; earlier drafts of this plan
named only v2 + v5 and undercounted v2's test count.)  Two ship
sites are plausible:

(a) **Inside `loft-libs-net` as a workspace integration crate.**
After extraction, the chunk repo is a Cargo workspace with one
member per library; add a sibling `tests-integration/` crate that
carries the harnesses.  CI runs `cargo test -p loft-libs-net-tests`
after the per-library steps.

(b) **Inside `lib/game_protocol/native/tests/`.**  Same shape as
Tier 2 — game_protocol is the topmost layer, the harnesses sit
where the surface is defined.  Requires adding a minimal
`lib/game_protocol/native/` crate (game_protocol has no native
binding today).

(a) is the cleaner long-term home — the harness *is* an integration
test of the chunk, not of any one library.  (b) is the shorter
migration path but couples the multiplayer suite to game_protocol's
extraction timing.

For now, prefer (a): ship the harnesses inside the `loft-libs-net`
external repo's `tests-integration/` crate at the same time the
chunk is re-cleaned (Phase 6r).

*Verify:* `cargo test --manifest-path tests-integration/Cargo.toml`
(or workspace `cargo test -p ...`) runs all 10 tests against the
checked-out `lib/server` + `lib/web` + `lib/game_protocol`;
deleting all three monorepo harnesses
(`tests/multiplayer_v{2,3,5}.rs`) leaves coverage intact.

**Order of operations.**  Tier 1 first (XS, unlocks gridmesh hygiene).
Tier 2 next (blocks Phase 5 graphics extraction).  Tier 3 last
(can land alongside Phase 6r since `loft-libs-net` is already
extracted; the integration suite is additive to that repo).

**Tier 3 blocker — cross-package `--native` link on Linux CI**
(surfaced 2026-05-31 during the `loft-libs-net` 6r/6.5 sweep, PR #2).
The omnibus first tried to lift the HTTP round-trip + WebSocket
echo tests from `lib/game_protocol/examples/` into
`server/tests/`.  Each test uses **both** `use server;` and
`use web;` in the same loft program — server to listen on a port,
web's http_get / ws_handler to drive a client arm via
`parallel { server_arm; client_arm }`.  Builds and runs locally
on macOS; fails on `ubuntu-latest` CI at the `rustc` link step
("linking with `cc` failed: exit status: 1") when both cdylibs
plus their transitive deps (ureq + rustls + ring from web,
TCP sockets from server) are pulled into one generated binary.
A server-only smoke (`listen` + `close`, no `web::`) passes —
the gate is specifically "two `#native` cdylibs from sibling
packages composed into one `loft --native test` binary."
**Filed** as @P389 in PROBLEMS.md.  Tests dropped from PR #2
(commit `c27198b`) and game_protocol-style two-process
multiplayer harnesses remain the path forward (Tier 3 above).
The gap reinforces option (a) for Tier 3 ship-site choice —
the workspace integration crate sidesteps the single-binary
limit by running clients and servers as separate processes.

**Tier 4 — `loft test --deps` — SHIPPED 2026-05-28.**  Consumer-side
walker that runs `loft test` on every dependency in the current
project's transitive (default) or direct tree.  Wired into the
canonical `library-ci.yml.example` template as a final step so a
chunk repo's PR catches "this graphics release broke gridmesh's
tests in our environment" before it merges, not after a downstream
consumer's CI flags it.

CLI surface (in `src/main.rs`):

```
loft test --deps                  # transitive — all deps + their deps
loft test --deps=direct           # one level only
loft test --deps=transitive       # explicit; same as plain --deps
```

`--deps` implies `--no-warnings` when running each dep's tests —
the consumer should not be blocked by lint debt inside a dep it
doesn't own.  Errors still surface via exit code.

Implementation status:

| # | What | Status |
|---|---|---|
| T1 | Free-fn dep resolver | implemented as local helper in `run_dep_tests` (path-dep + sibling fallback) |
| T2 | `--deps[=direct]` flag + direct walker | DONE — `run_dep_tests(transitive=false)` |
| T3 | Transitive walk + `HashSet<PathBuf>` cycle guard | DONE — default mode |
| T4 | `--lock=PATH` driver (read lockfile, resolve each pinned entry) | **DEFERRED** — registry-version deps fall through silently with a one-line warning to the host project; T4 closes that when lockfile parsing is wired |
| T5 | `--skip=` allow-list filter | DEFERRED — easy add when needed |
| T6 | `library-ci.yml.example` template `loft test --deps` step | DONE |

Smoke-tested via `lib/audience_crystal` (declares `gridmesh` as
path-dep): `loft test --deps=direct` ran 3 audience_crystal test
files + 4 gridmesh test files, reported `1 dep(s) tested, 0 failed`.

**Tier 5 — coverage gaps that never had a Rust home (S each, NEW).**

Validation run 2026-05-29 (every monorepo library exercised under
both `loft test` and `loft --native test`) surfaced four libraries
with **inadequate regression depth** that the original Phase-6t
framing missed.  Unlike Tiers 2–3, these gaps are *not* about
migrating coverage out of a Rust harness — the coverage **never
existed**.  Closing them is the work needed to ship extracted chunk
repos with real tests instead of smoke probes.

| Library | What `lib/<name>/tests/` carries today | Coverage gap | Blocks chunk |
|---|---|---|---|
| `imaging` | `tests/14-image.loft` doc-example + `tests/15-regression.loft` (9 tests, **DONE 2026-05-29**): `Pixel.value()` packing, save/load round-trip (4×4 + 8×3 non-square + 5×5 solid + 2×2 extremes), `(x,y) → y*w+x` addressing, `save_png` failure modes (0×0 image, nonexistent dir).  10 tests total green on both gates with `LOFT_DENY_WARNINGS=1`. | — | ~~Phase 5 (`loft-libs-graphics`)~~ unblocked |
| `world` | `tests/world.loft` smoke + `tests/02-persist.loft` (15 tests, **DONE 2026-05-29**): `chunk_idx_32`/`hex_idx_32` for positive AND negative inputs, `cell_count` (empty, after-set, overwrite, clear), `neighbour_count` (isolated + 6-axial-neighbours), `world_save`/`world_load` round-trip (empty, single-cell, many-cells-across-chunks, tick-preserved-through-`tick_and_decay`, negative-coords), `world_load` failure modes (missing file → 0, wrong magic → 0, wrong version → 0).  16 tests total, both gates green, `LOFT_DENY_WARNINGS=1` clean.  The MapFile JSON schema entry points (`world::load_mapfile` / `save_mapfile`) are still future work; covered when the schema migrates from `lib/moros_map`. | — | ~~Phase 6w (`loft-libs-world`)~~ unblocked for binary-format chunk extraction; MapFile schema landing is the only remaining 7a-step-4 work for full coverage |
| `server` | `tests/server.loft` — one `srv = listen(); srv.close()` smoke | Real surface (HTTP / WebSocket / TLS / session) only exercised by `multiplayer_v{2,3,5}.rs` (Tier 3).  Once Tier 3 lands in `loft-libs-net/tests-integration/`, server is covered transitively; `lib/server/tests/` itself remains a smoke (acceptable) | Phase 6 re-clean (6r) — **waived if Tier 3 lands first** |
| `markdown` | `tests/01-render.loft` — `fn main()` driver with `must_contain` / `must_not_contain` / `must_eq` helpers and 79 grouped assertions across ~25 feature areas (html_escape, slugify, rewrite_link, ATX/setext headings, paragraphs, bold/italic/strike/code, smart underscore, nesting, backslash escapes, images, links + titles, autolinks, hr, blockquote merging + separation, fenced code, lists UL/OL/continuation, task lists, tables + alignment, HTML-comment stripping, CRLF, UTF-8, raw-HTML escaping, tracker-tag autolinks, image URL rewriting, `extract_headings`).  Re-audit **2026-05-29**: this IS the ≥30-test coverage Tier 5 was supposed to add — the original Tier 5 framing was based on counting `fn test_*` (= 0) and missed the `fn main()` driver style. | — | ~~Markdown extraction (post-6w)~~ already covered; only a cosmetic refactor-to-`fn test_*`-discovery would remain |

*Target per library:* `cd lib/<name> && loft test` reports
≥10 test functions passing for `imaging` / `world` / `markdown`.
`server` is explicitly waived because Tier 3 covers it transitively.

*Order of operations.*  `imaging` first (blocks Phase 5 — the
soonest extraction that needs it; **DONE 2026-05-29** —
`lib/imaging/tests/15-regression.loft`, 9 tests + the doc-example
= 10 total green on both gates).  `world` next (pairs with the
MAPFILE entry-point landing in Phase 7a; co-blocks 6w).
`markdown` last (independent; no extraction blocker until
markdown ships externally).

*Why this Tier was missed originally:* the 2026-05-28 audit asked
"which Rust harnesses own library coverage?" — a *migration*
question.  It did not ask "which libraries lack adequate coverage
anywhere?" — a *creation* question.  Tier 5 closes the second one.

