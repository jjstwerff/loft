<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Library extraction — `lib/*/` → external repos

Move the `lib/*/` packages currently inside the main loft
repository out into per-family external GitHub repositories,
each consumable via the package registry.

This is the **execution arc** for **PKG.EXTRACT** in
[ROADMAP.md](../../ROADMAP.md).  The infrastructure work
(package registry MVP, lock file, format extensions) lives
in the sibling plan
[PACKAGES.md § Open work](../../PACKAGES.md#open-work).  This plan picks up
once the infrastructure ships.

## Status

**ACTIVE.**  Trigger (2026-05-23): @P321c diagnosis revealed
the project keeps re-adding library code to the compiler crate
(`src/codegen_runtime.rs` — `n_sha256` via @P321a, the
half-finished `n_load_png` / `n_save_png` attempts during
@P321c).  Activating this plan reframed the goal: instead of
adding MORE library code to the compiler crate, **drain what's
already there**.

**Progress to date (2026-05-24)**:

- **Phases 1 + 2 DONE.**  All library code drained from
  `src/native.rs` for crypto and web; `[native.functions]` is
  the declarative source of truth; `loft-ffi-build` generates
  `loft_register!` invocations from the manifest.
- **PKG.REG code complete** (PKG_REGISTRY.md R1-R9): `loft
  package`, `loft install`, `loft search`, `loft info`,
  lockfile, signing, recovery runbook.  Ecosystem bootstrap
  now follows the two-stage pattern from
  [PKG_REGISTRY.md § Two-stage bootstrap](../../PKG_REGISTRY.md#two-stage-bootstrap--interim-k_tmp--permanent-k_real):
  interim `K_tmp` on the dev laptop unblocks Phase 4
  immediately; YubiKey-backed `K_real` swap is a planned
  Scenario C rotation later.
- **Phase 3.5 — path-based dry-run** is in progress.
  - **3.5a DONE** (2026-05-24): `crypto` extracted from
    `lib/crypto/` to `../loft-crypto/`.  Full local CI
    passes (1 sandbox-only `index_hygiene_clean` failure,
    unrelated).  Hygiene gate keeps the 6 `n_crypto_*`
    symbols pinned via `FORBIDDEN_LIBRARY_SYMBOLS_MANUAL`.
    External `loft package` is reproducible (multiple runs
    → identical SHA-256).  Canonical extracted SHA:
    `1c68ce3624…` (5719 B).  **Finding logged**:
    `native/Cargo.toml` path-deps must be rewritten when
    a library moves (`../../../loft-ffi` → `../../loft/
    loft-ffi`), so cross-location SHAs naturally differ.
  - **3.5b TODO**: implement real path-dep resolution in
    `src/manifest.rs` + `src/parser/mod.rs`.  Inline
    `{ path = "X" }` is currently parsed as an opaque
    "version" string; the resolver finds deps only via
    sibling probe within `lib/`.  Required before
    extracting libraries WITH monorepo consumers.
    ~50 LoC + 1 unit test.
  - **3.5c TODO**: dry-run libraries that have monorepo
    consumers (`random`, `shapes`, `arguments`, `web`),
    blocked on 3.5b.
- **Phase 4 (real publish)** is no longer blocked on YubiKey
  arrival.  Bootstrap the registry with an interim `K_tmp`
  signing key on the dev laptop; ship the first real publish
  (crypto's GitHub repo + registry PR + signed index).  When
  YubiKeys arrive, rotate to `K_real` per
  [REGISTRY_RECOVERY.md § Scenario C](../../REGISTRY_RECOVERY.md#scenario-c).
  Constraint: do not ship a public loft release with `K_tmp`
  embedded until after the rotation to `K_real`.

When the bootstrap completes: per-library extraction proceeds
on its own validated schedule.  Some libraries may extract
early (stable, low-churn — crypto, random, shapes, arguments);
others stay in the monorepo until their API matures.

## Why a separate plan from PACKAGES.md infrastructure work

PACKAGES.md § Open work = INFRASTRUCTURE (registry, lock
file, format).  This plan = EXECUTION (which library
extracts when, how to migrate downstream consumers,
version-sync policy).

Different lifecycles:
- PACKAGES.md infrastructure work targets one focused arc
  (likely 0.8.6).
- This plan spans multiple releases — each library extracted
  on its own validated schedule.

Different acceptance criteria:
- Infrastructure work: "`loft install <name>` works,
  `loft.lock` honored, signing verifies."
- This plan: per-library — "lib/<X>/ removed from monorepo,
  `loft install <X>` from external repo produces identical
  behaviour, downstream consumers (other libraries, test
  scripts, examples) migrated."

Different audiences:
- PACKAGES.md readers care about how the registry / format
  works.
- This plan readers care about whether their favorite
  library is going to break or move.

## Current inventory — extraction candidates

The `lib/*/` packages with `loft.toml` (i.e., already using
the package format and ready in principle to extract once
the registry exists):

| Library | Notes | Likely extraction priority |
|---|---|---|
| `lib/arguments/` | CLI argument parsing | Early — small, stable, low-churn |
| `lib/crypto/` | Cryptographic primitives | Early — bounded scope |
| `lib/random/` | RNG | Early — bounded scope |
| `lib/shapes/` | Geometric primitives | Early |
| `lib/markdown/` | Markdown parser / formatter | Early — pure-loft, no native deps |
| `lib/imaging/` | Image manipulation | Mid — used by moros_*, validate dependency chain |
| `lib/graphics/` | OpenGL / 2D drawing | Mid — large, used by demos; coordinate with [`../02-graphics/`](../02-graphics/) plan |
| `lib/gridmesh/` | Hex / grid mesh generator | Mid — used by audience demo + dryopea; coordinate with [`../19-gridmesh/`](../19-gridmesh/) plan |
| `lib/server/` | HTTP server | Mid — coordinate with [`../08-server/`](../08-server/) plan; depends on game-loop additions |
| `lib/web/` | Web utilities | Mid |
| `lib/game_protocol/` | Multiplayer protocol | Mid — coordinate with EVENT_LOOP / multiplayer-editor / tic-tac-toe plans |
| `lib/world/` | Shared world model (sparse Cell/Chunk/World; expanding — see [§ Phase 7a](#phase-7a--split-moros-into-shared-world--moros-specific)) | Mid — consumed by TTT v5, audience demo, moros (post-split), dryopea |
| `lib/moros_editor/` | Moros editor (game-specific after world split) | Late — large, mid-development |
| `lib/moros_map/` | Moros map data — splits per [§ Phase 7a](#phase-7a--split-moros-into-shared-world--moros-specific) (palette / spawn stay; hex addressing + types move to `lib/world/`) | Late — paired with editor |
| `lib/moros_render/` | Moros rendering | Late — paired with editor |
| `lib/moros_sim/` | Moros simulation (collide.loft's geometry moves to `lib/world/` per Phase 7a) | Late — paired with editor |
| `lib/moros_ui/` | Moros UI | Late — paired with editor |
| `lib/audience_crystal/` | Audience-demo crystal mesh-gen prototype | **Stays in monorepo** (paired with the audience demo).  Phase 8 adds a `tests/` directory so it joins the library CI gates ([§ CI path](#ci-path-for-libraries-built-2026-05-23--travels-with-each-chunk)). |

Single-file `.loft` modules at `lib/*.loft` (`code.loft`,
`docs.loft`, `lexer.loft`, `logger.loft`, `overland.loft`,
`parser.loft`, `testlib.loft`, `wall.loft`) are NOT package-
format adopters yet.  Either keep as in-tree single-file
modules (no extraction), or migrate them to package format
first (own decision per file).

## Prerequisite — decouple the compiler crate from library code

Surveyed 2026-05-23.  The packages sort into three readiness
tiers based on where their native code physically lives.  The
goal of this prerequisite arc: **the loft compiler crate
contains zero library code or library blueprints — only the
language core (`default/*.loft` stays), runtime, codegen, and
stdlib symbols**.

### Stdlib vs library — the boundary (settled 2026-05-23)

**stdlib STAYS in the loft compiler crate; libraries are
extracted.**  This boundary is not subject to drain — it's the
permanent boundary between the language and its ecosystem.

| What's stdlib (stays in `src/`) | What's library (extracts to `lib/<X>/native/`) |
|---|---|
| `n_panic`, `n_assert` — fault primitives | `n_sha256` / `n_hmac_sha256` / `n_base64_*` — crypto |
| `n_log_info` / `_warn` / `_error` / `_fatal` — logging | `n_http_do` / `n_http_body` / web bridges — web |
| `n_json_parse` / `n_json_*` / JsonValue API — JSON (P54) | `n_rand` / `n_rand_seed` / `n_rand_indices` — random |
| `n_parallel_*` (rayon-backed) — threading primitives | `n_load_png` / `n_save_png` — imaging |
| `n_now`, `n_ticks` — clock | `n_sleep_ms` — sleep (web/time library) |
| `n_path_sep`, `n_hash_sorted` — runtime utility | OpenGL bindings (`gl_*`) — graphics |
| `n_stack_trace` — introspection | TCP / WebSocket / TLS — server / web |
| `n_get_store_lock` / `n_protect_store_frees` — concurrency runtime | `n_arguments` — CLI parsing (`lib/arguments`) |

Stdlib symbols are the ones every loft program could plausibly
need; library symbols are opt-in via `use <lib>`.  The drain
work below only targets the right column.  The `CODEGEN_RUNTIME_FNS`
registry in `src/codegen_runtime.rs` already mostly lists the
left column — the drain removes the right-column entries that
crept in (crypto via @P321a, random via @P321f) and prevents
new ones (@P321c was reverted along these lines 2026-05-23).

### Tier A — pure-loft, ready today (12)

`arguments`, `audience_crystal`, `game_protocol`, `gridmesh`,
`markdown`, `moros_editor`, `moros_map`, `moros_render`,
`moros_sim`, `moros_ui`, `shapes`, `world`.

No `#native` declarations at all.  The compiler already
resolves them via `probe_sibling_package` in
`src/parser/mod.rs`.  Mechanically extractable now; only
the registry consumption path (PKG.REG above) is missing.

### Tier B — clean native, blocked only on registry plumbing (4)

`graphics` (56 `#native` syms), `imaging` (2), `random` (3),
`server` (25).

Each owns a `native/` crate; the `n_*` implementations live
in `lib/<X>/native/src/` and are NOT duplicated in
`src/native.rs`.  The blueprint `pub fn ...; #native "n_xxx"`
declarations sit in the package's own `.loft` file.  Today
these are linked as workspace members; to leave the
monorepo entirely they need the cdylib loader
([PACKAGES.md `extensions.rs`](../../PACKAGES.md), designed
but not integrated).

### Tier C — leaking native, must drain first (2)

| Package | Leak | Action |
|---|---|---|
| `crypto` | 6/6 symbols (`n_sha256`, `n_hmac_sha256`, `n_base64_*`) live in `src/native.rs` | Create `lib/crypto/native/`, move the 6 fns out of the compiler crate |
| `web` | 13/19 symbols live in `src/native.rs` (web has a native dir, but only carries 6 of its 19 symbols) | Move the remaining 13 fns into `lib/web/native/src/lib.rs` |

Until these 19 hand-maintained `n_*` entries leave
`src/native.rs`, the compiler binary contains library Rust
code regardless of what happens elsewhere.  Independent of
PKG.REG and the cdylib loader — pure mechanical move per
package, no language change.

### Single-file `lib/*.loft` modules (8) — destinations decided

No `loft.toml` → no extraction path until converted or
folded.  Surveyed 2026-05-23; destinations:

| Module | Lines | Destination | Rationale |
|---|---|---|---|
| `wall.loft` | 60 | **Fold into `lib/world/`** (Phase 7a) | Mandatory for moros + dryopea (user-built walls + rock faces) — first-class part of the world surface, not a separate package |
| `overland.loft` | 7 | **Fold into `lib/world/`** (Phase 7a) | `OverlandMap` enum (material / item / height / water) — terrain layer data, world-handling territory |
| `logger.loft` | 34 | **Stays as-is alongside self-hosting cluster** (Q #11 resolved 2026-05-24, option (c)) | Single consumer `lib/parser.loft` (also monorepo-internal); promotion to standalone `lib/logger/` package deferred until a non-parser consumer emerges |
| `code.loft` | 263 | **Stays in monorepo as self-hosting tooling** | Loft type / field metadata (`Field`, `Type` structs) — used by the self-hosting parser; not a library consumers want |
| `lexer.loft` | 477 | **Stays in monorepo as self-hosting tooling** | Loft source lexer — used by the self-hosting parser and `gendoc` |
| `parser.loft` | 674 | **Stays in monorepo as self-hosting tooling** | Loft source parser — `use lexer; use logger; use code;` |
| `docs.loft` | 21 | **Stays in monorepo as build tooling** | Used by `gendoc`; depends on `lexer` |
| `testlib.loft` | 57 | **Stays in monorepo as test infrastructure** | Used by `tests/docs/17-libraries.loft`; monorepo-test-only consumer |

**Counts after Phase 1c (final, audit-confirmed 2026-05-24):**
**0 standalone conversions** (Q #11 closed to option (c) —
`logger.loft` stays as-is, only consumer is `lib/parser.loft`
which itself stays); **2 folds into `lib/world/`** (`wall.loft`,
`overland.loft`) deferred to Phase 7a; **6 stay as
monorepo-internal tooling** (`logger.loft`, `code.loft`,
`lexer.loft`, `parser.loft`, `docs.loft`, `testlib.loft`).

The self-hosting cluster (`code` + `lexer` + `parser` +
`docs`) could form an optional 6th chunk `loft-libs-self` if
external consumers ever want a programmatic loft parser, but
the current call sites are all monorepo-internal — defer
that decision until a real external consumer appears.

### Dynamic native-registry path

Today `src/native.rs::NATIVE_TABLE` is a static
`&[(&str, fn)]` slice — every library symbol must be
hand-maintained there.  After the leak drain, the next
step is replacing the static table with a registry
populated by package manifests:

- **Compile-time aggregator** (first step).  `build.rs`
  walks `lib/*/loft.toml::[native.functions]` and emits
  the registry as generated code that links each
  package's `native/` crate.  Removes hand-maintenance;
  packages still live in the monorepo.  This is the
  smallest change that achieves "compiler crate carries
  zero library code."
- **`dlopen` loader** (follow-on).  Optional once
  external registries land — uses the `extensions.rs`
  design from PACKAGES.md to resolve native symbols at
  install time from a downloaded `.rlib` / `.so`.  Not
  required for the monorepo-internal goal.

### Work order

Phases 1-2 ([§ Phases](#phases--detailed-execution-plan)
below) own the decoupling work; phase 3 waits on PKG.REG;
phases 4-7 extract each chunk.  The decoupling phases
land independently from PKG.REG — they don't change
user-visible behaviour.

## Chunk grouping — a few repos, not 17

One GitHub repo per library = 17 repos to track, release, and CI separately.
To avoid that sprawl the libraries extract in **chunks**: a small number of
multi-package repos, each holding a related FAMILY that versions + releases
together under one CI workflow.  Each chunk is a workspace of `loft.toml`
packages (published to the registry per-package, maintained as one repo).

Proposed chunks (refine before the first extraction):

**Library chunks** (libraries-only repos, registry-published):

| Chunk repo | Packages | Rationale |
|---|---|---|
| `loft-libs-core` | `arguments`, `random`, `crypto` (plus future `json` / `html` / `fs` drains from stdlib — see [§ Phase 3.6 stdlib drain](#phase-36--stdlib-drain-into-libs)) | Small, stable, no graphics deps — extract first |
| `loft-libs-graphics` | `graphics`, `imaging`, `gridmesh`, `shapes` | Graphics stack + `#native` crates; `shapes` is 2D shape drawing + collision (was in core, moved here 2026-05-24 — its loft.toml comment is "Shape drawing and 2D collision detection library"); coordinate with [`../02-graphics/`](../02-graphics/) |
| `loft-libs-net` | `server`, `web`, `game_protocol` | HTTP / multiplayer; coordinate with [`../08-server/`](../08-server/) |
| `loft-libs-world` | `world` (expanded by Phase 7a to absorb moros's shared spatial primitives: hex addressing, wall geometry, groups, height, coupled geometry).  Folds in `lib/wall.loft` content rather than carrying `wall` as a separate package. | Shared map / spatial primitives consumed by TTT v5, audience demo, moros, dryopea ([@PLAN46](../../plans/future/46-dryopea/README.md)).  Lives in its own chunk because consumers span multiple games and the dryopea plan blocks on it. |

**Game / application repos** (host game-specific libraries
AND the playable game; registry-publish OPTIONAL — these
exist primarily as consumers of the library chunks above):

| Repo | Hosts | Status |
|---|---|---|
| **`moros`** *(existing GitHub project — reuse)* | `moros_editor`, `moros_map` (game-only remnant after Phase 7a), `moros_render`, `moros_sim`, `moros_ui` + the playable game executable | Active development; depends on `loft-libs-graphics` + `loft-libs-world`.  Moros's chunk doesn't ship as a separate `loft-moros` repo — the libraries live alongside the game in the existing moros project, which keeps game + its specific libraries colocated for the maintainer.  Per-library tarballs can still publish to the registry (each `lib/moros_*/` has its own `loft.toml`), but the **homepage is the moros repo** rather than a chunk-libraries repo.  **Forthcoming arc (2026-05-24):** the existing moros project is mostly JavaScript today; the maintainer plans to migrate it to loft over time.  Phase 7b moves the EXISTING loft `lib/moros_*/` work into the moros repo; the JS-to-loft port of the game itself is a sibling track inside the moros repo (not part of plan-12). |
| **`dryopea`** *(new project to create — [@PLAN46](../../plans/future/46-dryopea/README.md))* | Dryopea-specific libraries + the playable game executable | New project, same model as moros: game + its game-specific libraries colocated in one repo, with `loft-libs-world` as the shared dependency from the registry.  Created when @PLAN46 starts execution. |

`lib/audience_crystal/` (the projector's crystal mesh-gen prototype that
`gridmesh` was extracted from) is NOT an extraction candidate — it stays
in-monorepo with the audience demo.  It also has no package `tests/` dir, so it
is the one packaged lib NOT covered by the library gates below; its behaviour is
gated via the core `tests/scripts/130|133|135` cross-mode equivalence tests.
Adding package tests (so it joins the gate) is the small hardening follow-up.

A chunk extracts as a unit (one `git filter-repo` / `subtree split` of its
`lib/*` dirs), keeping intra-chunk deps as path deps and cross-chunk deps (e.g.
moros → graphics) as registry deps.  The
[extraction template](#per-library-extraction-template) applies per-chunk
(step 6 deletes the whole chunk's `lib/*` dirs in one PR).

**Why few chunks, not 17 repos.**  17 `loft-<lib>` repos =
17 CI workflows to maintain, 17 release cadences, 17
README.md files for users to grep through when they want
"where does `lib/graphics/` live now?"  A user who knows
"graphics is a graphics chunk" can find it; a user staring
at 17 repos can't.  Four chunks is the cap.  New libraries
join an existing chunk by family fit, not a new repo.

### Cross-chunk dependency graph

Inter-chunk deps resolve via the registry (Stage A
publishes; Stage B consumes).  Intra-chunk deps stay as
path deps inside the chunk repo.  Surveyed 2026-05-23 from
existing `loft.toml` files + `use` statements; treat as a
TARGET state — must be re-verified at each chunk's
Stage-A1 step before extracting.

```
loft-libs-core   (no chunk deps — leaves of the graph)
  ↑
  ├── loft-libs-graphics   (arguments from core; shapes / gridmesh internal)
  │     ↑
  │     └── loft-libs-world   (shapes / gridmesh from graphics IF
  │           ↑                 world re-uses 2D collision or mesh primitives
  │           │                 — verify in 7a)
  │           │
  ├── loft-libs-net        (crypto / arguments from core)
  │
  ├── moros (existing project, reused)
  │       depends on graphics + world (and possibly net for
  │       multiplayer demos); hosts moros_* libraries AND the
  │       moros game.
  │
  └── dryopea (new project — @PLAN46)
          depends on world (shared world primitives); hosts
          dryopea-specific libraries AND the dryopea game.
```

**Rules this graph imposes on phase ordering:**

- Phase 4 (`loft-libs-core`) must publish first — every
  other chunk depends on it.
- Phase 5 (`loft-libs-graphics`) and Phase 6 (`loft-libs-net`)
  are independent of each other and can interleave.
- Phase 6w (`loft-libs-world`) needs `loft-libs-core`
  published, plus Phase 7a complete in the monorepo so the
  hex / wall / overland surface is finished.  May also
  need graphics published if the world library uses
  gridmesh primitives — to be confirmed in Phase 7a.
- Phase 7b (moros — existing project) needs both `loft-libs-graphics`
  and `loft-libs-world` published.

**Open verification:** during Phase 1 (decoupling), each
chunk's actual `use` statements get audited and the graph
above gets confirmed or revised.  Notable unknowns:
- Does `lib/world/` (post-7a) need `gridmesh` from
  `loft-libs-graphics`, or are the geometries independent?
- Does the `moros` project need `loft-libs-net` for `game_protocol`
  (multiplayer-editor / tic-tac-toe consumers), or do those
  demos live elsewhere?

These verifications block the relevant phase from
starting Stage A; the answer determines whether a chunk
needs another chunk's registry version in scope.

## Phases — detailed execution plan

Each phase has a concrete acceptance criterion.  Phases 1-2
can land in parallel with each other and with PKG.REG.
Phases 4-7 serialize (one chunk at a time — minimises
downstream consumer churn and proves the per-chunk template
on the smallest chunk before larger ones commit).

**One chunk per release window — not a sprint.**  Standing
up a new GitHub project costs real admin work: org
permissions, branch protection, CI secrets, release
tagging conventions, README + LICENSE + CONTRIBUTING,
issue templates, the first round of registry-publish
diagnostics.  Doing five at once is overload.  The plan
deliberately splits this across **five separate phases**
(4, 5, 6, 6w, 7b — one chunk extraction each), so each
chunk gets its own release window with soak time before
the next opens.

Recommended soak between chunks: at least one minor
release.  The point isn't a fixed calendar — it's that
each new chunk repo gets a real consumer round-trip
through `loft install <pkg>` before the next admin batch
starts.

### Phase 1 — Drain `src/native.rs` of library symbols

Owned by this plan, no external dependency.  Moves library
Rust out of the compiler crate so later phases can extract
cleanly.

| Sub | Work | Effort | Status |
|---|---|---|---|
| 1a | Move 6 fns (`n_sha256`, `n_hmac_sha256`, `n_hmac_sha256_raw`, `n_base64_encode`, `n_base64_decode`, `n_base64url_encode`) from `src/native.rs` AND the @P321a duplicates in `src/codegen_runtime.rs` into new `lib/crypto/native/` cdylib.  Also moves `src/sha256.rs` out (only crypto used it); `src/base64.rs` stays (compiler-internal `main.rs::wasm_b64` use for `--html` export) but a copy ships with `lib/crypto/native`. | XS | **DONE 2026-05-23** — `lib/crypto/native/{Cargo.toml,src/{lib.rs,sha256.rs,base64.rs}}` ships the 6 fns via the standard `loft_ffi::loft_register!` ABI; `lib/crypto/loft.toml` declares `native = "loft_crypto"`; `is_crypto_runtime_symbol` removed from `src/generation/mod.rs`; both interp + `--native` crypto tests pass 6/6. |
| 1b | Drain the 19 `lib/web` `n_*` symbols (`n_http_do`, `n_http_body`, `n_ws_connect`, `n_ws_client_send`, `n_ws_client_send_binary`, `n_ws_client_recv`, `n_ws_client_message`, `n_ws_client_opcode`, `n_ws_client_close`, `n_sleep_ms`, `n_pack_reset`, `n_pack_u8`, `n_pack_u16_le`, `n_pack_u32_le`, `n_pack_take`, `n_byte_at`, `n_ws_group_clear`, `n_ws_group_add`, `n_ws_group_poll`) from the compiler crate's regular dispatch path.  Lock in via `FORBIDDEN_LIBRARY_SYMBOLS`. | XS | **DONE 2026-05-24** — `lib/web/native/src/lib.rs` already ships all 19 via `loft_ffi::loft_register!`; `lib/web/loft.toml` declares `native = "loft_web"`.  Regular native path uses the cdylib at runtime via `extensions::wire_native_fns`.  The 13 WASM-bridge stubs in `src/native.rs::WEB_FUNCTIONS_WASM` (gated on `#[cfg(all(target_arch = "wasm32", feature = "wasm"))]`) stay — WASM has no `dlopen`, so the only way to register native symbols is statically.  `tests/extraction_hygiene.rs` was extended with `wasm32_cfg_gated_lines` to skip lines inside `#[cfg(...wasm32...)]` blocks; the 19 web symbols now appear in `FORBIDDEN_LIBRARY_SYMBOLS` and the test passes.  Decoupling the WASM bridge itself (so `lib/web/native/` can also compile a `wasm32-unknown-unknown` cdylib + the host bridge moves out of `src/wasm.rs`) is a separate future phase — not blocking the per-library external-repo extraction below. |
| 1c | Resolve the 8 single-file `lib/*.loft` modules per [§ Single-file modules — destinations decided](#single-file-loftloft-modules-8--destinations-decided): convert at most 1 (`logger.loft`, pending Open Q #11) to package format; the other 7 either fold into `lib/world/` in Phase 7a (`wall.loft`, `overland.loft`) or stay as monorepo-internal tooling (`code.loft`, `lexer.loft`, `parser.loft`, `docs.loft`, `testlib.loft`). | XS-S | **DONE 2026-05-24** — consumer audit (`grep -rln 'use <module>'` across `lib/`, `tests/`, `tools/`, `default/`) confirmed each destination.  Per-module consumer counts: `logger` 1 (`lib/parser.loft`), `code` 1 (`lib/parser.loft`), `lexer` 3 (`lib/parser.loft`, `lib/docs.loft`, `tests/docs/15-lexer.loft`), `parser` 1 (`tests/docs/16-parser.loft`), `docs` 0 — standalone `fn main()` runnable, `testlib` 1 (`tests/docs/17-libraries.loft` — `lib/testlib.loft` references appear to be the file itself, not a consumer), `wall` 0, `overland` 0.  **Open Q #11 resolved to option (c)**: `logger.loft` stays as-is alongside `lib/parser.loft` — only known consumer is `parser.loft` which itself stays as monorepo self-hosting tooling; no second consumer emerged.  No code moves needed for 1c — the 5 self-hosting / build-tooling files (`code` + `lexer` + `parser` + `docs` + `testlib`) keep their `lib/` location with their non-extractable status documented in the destinations table; the 2 fold-into-`lib/world/` files (`wall`, `overland`) stay parked at `lib/*.loft` until Phase 7a's `lib/world/` reorganisation. |

**Acceptance gate — `tests/extraction_hygiene.rs`.**  Every Phase 1
sub-task ends by adding its drained `n_*` symbol names to the
`FORBIDDEN_LIBRARY_SYMBOLS` list in `tests/extraction_hygiene.rs`.
The test:

1. Walks `src/**/*.rs`, skips `//` comments, fails if any code
   position contains a listed symbol name.  Word-boundary match
   so `n_sha256` matches but not `not_n_sha256_oops` etc.  Lines
   inside `#[cfg(...wasm32...)]` gated blocks are exempted via
   `wasm32_cfg_gated_lines` (Phase 1b, 2026-05-24): WASM has no
   `dlopen`, so its host-bridge stubs (`src/wasm.rs::host_*` →
   `src/native.rs::WEB_FUNCTIONS_WASM`) MUST live in the compiler
   crate until the WASM bridge architecture is decoupled.  Regular
   native dispatch uses the `lib/<X>/native/` cdylib via
   `extensions::wire_native_fns`.
2. Reads the main crate's `Cargo.toml`, fails if it lists any
   library-only dep from `FORBIDDEN_MAIN_CRATE_DEPS` (currently
   empty — filled in as cdylib loading lets libraries' deps move
   to their own `native/Cargo.toml` instead of the main one).

This is the **finisher**: each drain sub-task is "complete" when
the symbol(s) appear in `FORBIDDEN_LIBRARY_SYMBOLS` AND the test
passes.  The list itself is the audit trail — searching for a
`n_*` name in the test file tells future contributors "this
symbol used to live in the compiler crate and was drained in
phase X".  A future @P321-style attempt to add `n_load_png` /
`n_save_png` (or any other library symbol) to `src/codegen_runtime.rs`
fails CI on this gate.

Stdlib symbols (`n_panic`, `n_assert`, `n_log_*`, `n_json_*`,
`n_parallel_*`, `n_now`, `n_ticks`, `n_path_sep`, `n_hash_sorted`,
`n_stack_trace`, `n_get_store_lock`, `n_protect_store_frees`) are
deliberately NOT in the list — they stay in the compiler crate by
design (see [§ Stdlib vs library — the boundary](#stdlib-vs-library--the-boundary-settled-2026-05-23)).

**Backstop acceptance for the whole phase:** `grep '^fn n_'
src/native.rs` returns only symbols backing `default/*.loft` and
runtime helpers (no library symbols); all `lib/*/` directories
have `loft.toml`.  The automated gate above is the day-to-day
check; this manual check is the audit form.

### Phase 2 — Compile-time native-registry aggregator

**DONE 2026-05-24** across three sub-steps (1 → 2 → 3 below).

Original goal: replace the hand-maintained static
`NATIVE_TABLE` slice in `src/native.rs` with
`build.rs`-generated registration that walks
`lib/*/loft.toml::[native.functions]` and emits the table
at compile time.

**Final state for the two drained libraries** (crypto, web):

* Manifest is the single declarative source of truth for which
  loft fn maps to which native `n_*` symbol
  (`lib/<X>/loft.toml::[native.functions]`).
* Parser populates `def.native`, `native_symbols`, and
  `native_symbol_crates` from the manifest at parse time
  (both legacy `apply_manifest_side_effects` and
  sibling-probe `register_native_manifest` paths).
* Native codegen reaches the same dispatch via the
  `def.native` + crate-qualifier path; `native_symbols` is
  the fallback for libraries that declare `[native.functions]`
  without an `[library] native` stem.
* `lib/<X>/native/build.rs` generates the
  `loft_register!` macro invocation from the manifest, so
  the cdylib's symbol list isn't duplicated.

**Acceptance check satisfied:** adding or removing a
crypto / web library's `[native.functions]` entry IS the
only change required — the build.rs regenerates the
register list, the parser repopulates the bindings, the
hygiene gate auto-updates its forbidden list.
`src/native.rs::FUNCTIONS` (stdlib NATIVE_TABLE) is
intentionally NOT touched — that table is stdlib, not
libraries; the stdlib-vs-library boundary table makes the
split explicit.

**Effort:** M (~600 lines net across parser + codegen +
build.rs + plan README updates).

#### Phase 2 step 1 — `[native.functions]` as the single source of truth (DONE 2026-05-24)

The simpler half of Phase 2 — making `[native.functions]`
in `lib/<X>/loft.toml` the declarative source of truth for
which native `n_*` symbols a library owns — landed first.
The `Manifest` reader at [`src/manifest.rs:64`](../../../src/manifest.rs)
already parses the section (and `src/parser/mod.rs:4392`
already populates `data.native_symbols` from it); the gap
was that no library actually declared its functions.

Populated 2026-05-24:

* `lib/crypto/loft.toml::[native.functions]` — 6 entries
  (the phase 1a drain).
* `lib/web/loft.toml::[native.functions]` — 19 entries
  (the phase 1b drain).

`tests/extraction_hygiene.rs` reworked to derive its
`FORBIDDEN_LIBRARY_SYMBOLS` list from these manifests at
test time (via a new `forbidden_library_symbols()` walker
that reads every `lib/*/loft.toml`).  The previous
hand-maintained const shrank to the empty
`FORBIDDEN_LIBRARY_SYMBOLS_MANUAL` slot, reserved for
libraries whose manifest can't yet declare the symbol.
Added `manifest_native_functions_cover_drained_libraries`
test that asserts crypto's 6 + web's 19 symbols are visible
through the manifest path — guards against silent loss of
`[native.functions]` sections via manifest typos / accidental
deletion.

#### Phase 2 step 2 — `[native.functions]` is now load-bearing (DONE 2026-05-24)

The redundant `#native "symbol"` annotations in
`lib/<X>/src/<name>.loft` are no longer required for the
two drained libraries.  `src/parser/mod.rs` was extended to
populate `def.native` from `[native.functions]` in BOTH
the legacy `apply_manifest_side_effects` path AND the
sibling-package `register_native_manifest` path (the
latter is hit when a script reaches a library via the
ancestor-walk probe — e.g., `lib/game_protocol/examples/`
reaching `lib/web` without `--lib`).

Both interpreter and `--native` paths now route through
the manifest:

* **Interpreter**: `wire_native_fns` / `register_native_stubs`
  / the bytecode emitter at `src/state/codegen.rs:2207`
  all read `def.native`.  Once populated from the manifest,
  the dispatch is identical to the `#native`-annotated
  version.
* **Native codegen**: `src/generation/mod.rs:2004` consults
  `data.native_symbols` directly (the manifest map) AS
  WELL AS `def.native` — both paths reach the same Rust
  symbol.

Sibling probe regression caught + fixed: the multiplayer
v2/v3/v5 test suites spawn TTT server + client subprocesses.
The client uses `lib/web` reached via the sibling probe
(no `--lib` flag).  The first cut populated
`def.native` only in the legacy path; the client's web
symbols stayed empty → "no MAP after 500 polls" failures.
Adding the same population to `register_native_manifest`
restored all multiplayer suites (v2/3/3 + v5/t1-t5 all
green).

`#native` annotations remain in the loft source for now —
removing them is a one-line `sed -i '/^#native "n_/d'`
per library plus a regression-test pass; not done in this
commit because the user-facing test suites were the
priority guard.  Once the wasm bridge decouples (see
phase 1b note), `lib/imaging` and friends can follow the
same path.

#### Phase 2 step 3 — `build.rs` codegen for `loft_register!` (DONE 2026-05-24)

The last gap: `lib/<X>/native/src/lib.rs`'s
`loft_ffi::loft_register! { … }` block still hand-maintained
a list of symbol names duplicating
`lib/<X>/loft.toml::[native.functions]`.  Closed by adding a
small `build.rs` to each library's native crate that:

1. Reads `../loft.toml` (the package manifest one level up).
2. Walks `[native.functions]` line-by-line (same minimal
   scanner shape as `src/manifest.rs::read_manifest` —
   build.rs can't depend on the loft crate without going
   circular).
3. Generates `$OUT_DIR/loft_register_gen.rs` containing
   `loft_ffi::loft_register! { <values> }`.
4. `src/lib.rs` does
   `include!(concat!(env!("OUT_DIR"), "/loft_register_gen.rs"));`
   at module scope — the macro expands in the normal compilation pass.

Added files (initial cut):

* `lib/crypto/native/build.rs` (~60 lines)
* `lib/web/native/build.rs` (~60 lines; near-duplicate of crypto's)

**Option A collapse — DONE 2026-05-24.**  The two duplicated build
scripts were collapsed into a shared `loft-ffi-build` crate at the
repo root (sibling of `loft-ffi`).  Each library's `build.rs` is now
two lines:

```rust
fn main() {
    loft_ffi_build::generate_register_invocation("../loft.toml");
}
```

…plus a single `[build-dependencies]` row in the library's
`native/Cargo.toml`:

```toml
[build-dependencies]
loft-ffi-build = { path = "../../../loft-ffi-build" }
```

The TOML scanner + register-invocation emitter live in one place
(`loft-ffi-build/src/lib.rs::generate_register_invocation` +
`parse_native_functions`).  Three unit tests in the helper cover
the section-scanner edge cases (parses the section / empty section /
no section).

Adding a new library that owns native symbols is now:

1. `lib/<X>/loft.toml::[native.functions]` — declare the rows.
2. `lib/<X>/native/src/lib.rs` — write the
   `pub unsafe extern "C" fn n_*` bodies.
3. `lib/<X>/native/build.rs` — the two-line delegation above.
4. `lib/<X>/native/Cargo.toml` — `loft-ffi-build` build-dep row.

Steps 3 + 4 are exact copy-paste between libraries; only step 1 +
step 2 carry library-specific content.

Removed lines:

* `lib/crypto/native/src/lib.rs` — 7 lines (`loft_register! { 6
  symbols }`).
* `lib/web/native/src/lib.rs` — 20 lines (`loft_register! { 19
  symbols }`).

Now adding a new native symbol to crypto/web is exactly **two
edits**:

1. `lib/<X>/loft.toml::[native.functions]` — add the
   `loft_name = "n_symbol"` row.
2. `lib/<X>/native/src/lib.rs` — add the
   `pub unsafe extern "C" fn n_symbol(...)` body.

The `loft_register!` symbol list is generated.  The
extraction-hygiene gate's forbidden list is generated.  The
loft compiler's `def.native` + `native_symbols` +
`native_symbol_crates` populations are all manifest-driven.
The hand-maintained surfaces collapse to: the manifest row,
the cdylib fn body, and the loft fn declaration (which still
exists to give the cdylib symbol its loft-side type
signature — the compiler reads that signature to marshal
args and route the call).

Proof: empirically removing `pack_take = "n_pack_take"` from
`lib/web/loft.toml` and rebuilding regenerated the
`loft_register_gen.rs` file WITHOUT `n_pack_take`.  Restoring
the line and rebuilding restored it.  The
`cargo:rerun-if-changed=../loft.toml` directive ensures
manifest edits trigger regeneration.

### Phase 2 — Status

**FULLY DONE 2026-05-24** (acceptance criterion at
`#phase-2--compile-time-native-registry-aggregator` met for
the two drained libraries: crypto + web).

What deferred future work remains (outside Phase 2's scope):

1. **`src/native.rs::FUNCTIONS` (the stdlib NATIVE_TABLE)**
   stays hand-maintained — `[native.functions]` is for
   libraries, not stdlib.  See the stdlib-vs-library
   boundary table.
2. **Other libraries** (`imaging`, `random`, `graphics`,
   `server`, …) can adopt the same pattern when their
   drains land.  Today the build.rs lives only in the two
   drained libraries.
3. **WASM bridge decoupling** — unblocks @P321c imaging and
   removes the WASM-cfg exemption.  Separate future phase.
4. ~~**Shared `loft-ffi-build` helper crate**~~ — DONE
   2026-05-24 (Option A above); the two near-identical
   build scripts collapsed into a 60-line library at
   `loft-ffi-build/src/lib.rs`.

### Phase 3 — Coordinate with PKG.REG

The full registry-backed publish flow (Phase 4+ "Stage B")
needs:
- **PKG.REG MVP** ships AND a trust-root keypair is generated
  + embedded in the maintainer's local loft build.  Code is
  DONE 2026-05-24 (PKG_REGISTRY.md R1-R9).  Two-stage
  ecosystem bootstrap unblocks Phase 4 now: interim `K_tmp`
  on the dev laptop is sufficient for `loft install` to
  verify against the local build (do not ship a public loft
  release with `K_tmp` embedded — see
  [PKG_REGISTRY.md § Two-stage bootstrap](../../PKG_REGISTRY.md#two-stage-bootstrap--interim-k_tmp--permanent-k_real)).
- **cdylib loader** (`extensions.rs`) — already shipped via
  Phase 2.  This bullet was originally about external-
  registry-installed packages contributing native symbols at
  install time; Phase 2's `[native.functions]` + build.rs
  pattern handles this cleanly.  No additional work.

**Acceptance** (when both above land): `loft install <name>`
resolves a published package; `#native` symbols dispatch
through the cdylib loader; `make ci` passes against a
registry-installed copy of one test package.

### Phase 3.5 — Path-based dry-run (unblocks NOW)

**Goal**: validate the Stage-A extraction mechanics
(move library out of monorepo, monorepo consumes via
external path) **before** the live registry exists.
Catches hidden assumptions ("oh, this file path was
hardcoded somewhere"), exposes consumer-side bugs in
`loft.toml` path-dep handling, and gives confidence the
real publish (Phase 4+) is mechanical rather than a
discovery exercise.

**Why this is unblocked (in stages)**:

- **3.5a — libraries WITHOUT monorepo consumers** (crypto):
  the dry-run is a pure "remove + verify" operation; nothing
  in monorepo references the library so the `path = "..."`
  resolution path is not exercised.  No parser changes
  required.
- **3.5b — implement real path-dep resolution**:
  Discovered during the 3.5a dry-run for `crypto`
  (2026-05-24): the inline `{ path = "X" }` syntax in
  existing `lib/*/loft.toml` files is **decorative** — the
  manifest parser stores the whole string as the dep
  "version" and the resolver finds deps only via sibling
  probe within `lib/` (registered as a `lib_dir` from the
  `--lib lib` cmdline).  Real path-deps that point OUT of
  `lib/` need parser support: extract the `path` field,
  resolve it relative to the manifest's package dir, and
  register the parent of that resolved path as a `lib_dir`.
  ~50 LoC change in `src/manifest.rs` + `src/parser/mod.rs::
  apply_manifest_side_effects` + 1 unit test.
- **3.5c — libraries WITH monorepo consumers** (random,
  web, shapes, …): blocked on 3.5b.  Each consumer's
  `loft.toml` flips to `dep = { path = "..." }`; resolver
  walks the new location.

When the registry ships, flipping `path = ".."` →
`version = "0.1.0"` is a one-line per-consumer change in
each `loft.toml`.

**Steps for the first dry-run target (`crypto`)**:

1. Pick a sibling location for the external copy.  Two
   options: a real `loft-lang/loft-crypto` GitHub repo
   (more realistic) OR a sibling directory
   `../loft-crypto/` under your dev tree (faster to set
   up; works fine).  Use the GitHub repo if you want to
   exercise the per-library CI shape from Phase 4's
   "Additional deliverables" early.
2. Move `lib/crypto/` to the new location.  Preserve the
   directory tree exactly — `loft.toml`, `src/`,
   `native/`, `tests/`, `README.md` (if any).  The
   already-shipped Phase 2 `[native.functions]` + build.rs
   travel with the package.
3. For each monorepo consumer that uses `use crypto;`,
   update `loft.toml`:

   ```toml
   [dependencies]
   crypto = { path = "../loft-crypto" }
   ```

4. Run `make ci`.  Validates:
   - The path-dep resolves cleanly.
   - `loft install` / dep-walk doesn't choke on a non-
     `lib/`-prefixed source location.
   - Native cdylib (`libloft_crypto.dylib`) builds from
     the external location.
   - Extraction-hygiene gate still passes (the 6
     forbidden crypto `n_*` symbols are still absent from
     `src/**/*.rs`).
5. Run `loft package` in the external `loft-crypto/`
   directory to confirm the tarball builds cleanly from
   the external location.

   **3.5a finding (2026-05-24)**: the tarball's SHA-256
   is location-dependent because `native/Cargo.toml`
   path-deps must be rewritten when the library moves
   (`../../../loft-ffi` → `../../loft/loft-ffi`).  Both
   tarballs build, but the SHA-256 differs:

   | Location | path-dep | SHA-256 | size |
   |---|---|---|---|
   | `lib/crypto/` (monorepo) | `../../../loft-ffi` | `43ebf109b020…` | 5717 B |
   | `../loft-crypto/` (extracted) | `../../loft/loft-ffi` | `1c68ce3624…` | 5719 B |

   **Implication for Phase 4 publish**: the canonical
   distribution SHA is whatever the LIBRARY'S OWN REPO
   produces — `lib/crypto/`'s SHA is not portable.
   Publish-time, the external repo's `native/Cargo.toml`
   should either (a) replace path-deps with version-deps
   (`loft-ffi = "0.1"`) for the published manifest, or
   (b) normalize path-deps to a canonical form before
   hashing.  Recorded as **Phase 4 design item**.

**Acceptance**:
- `lib/crypto/` removed from monorepo.
- `loft.toml`s of consumers reference the external path
  (none for crypto — no consumers in monorepo).
- Full local CI passes (`./scripts/find_problems.sh --wait`
  modulo the recurring sandbox-only `index_hygiene_clean`).
- `loft package` in the external location produces a
  reproducible tarball (multiple runs → identical
  SHA-256).

**What this does NOT do**:

- Doesn't publish anything (no GitHub release, no
  registry PR).  Output is just "we know the mechanic
  works."
- Doesn't ship a loft release.  This is all internal
  dev-tree movement.
- Doesn't decouple monorepo CI from the external repo —
  during the dry-run, the monorepo + external dir must
  travel together.  Real Phase 4 fixes this by going
  through the registry instead of `path =`.

**When to flip dry-run → real publish (later)**:

The interim `K_tmp` bootstrap removes the YubiKey wait — Phase 4
can start as soon as the dry-run for a library completes:

1. Generate `K_tmp` on the dev laptop (`loft-keygen generate`).
   Embed its public key in `src/registry_keys.rs`.  Build a
   local loft binary that trusts `K_tmp`; do NOT ship this
   binary publicly.
2. Create `loft-lang/loft-crypto` GitHub repo from the
   extracted dry-run dir.  Bootstrap `loft-lang/registry` per
   [REGISTRY_BOOTSTRAP.md](../../REGISTRY_BOOTSTRAP.md)
   (signing with `K_tmp`).
3. Tag `v0.1.0` in `loft-crypto`.  `gh release create v0.1.0
   crypto-0.1.0.tar.gz`.
4. Open the registry PR adding the version row.  Sign + merge
   per PKG_REGISTRY.md.
5. Update consumer `loft.toml`s: `path = ".."` →
   `crypto = ">=0.1"`.  Test `loft install crypto` against the
   live registry from the K_tmp-trusting local loft.

After YubiKeys arrive:

6. Generate `K_real` on the YubiKey-backed setup.
7. Run [REGISTRY_RECOVERY.md § Scenario C](../../REGISTRY_RECOVERY.md#scenario-c)
   as a planned rotation — re-sign every published
   `index.json.sig` with `K_real`, embed `K_real` in
   `TRUSTED_PUBLIC_KEYS`, ship the first public loft release.

**Effort**: S (~half day) for crypto.  Subsequent
dry-runs (random, shapes, arguments, web) follow the same
template; each is faster as the muscle memory builds.
Recommended order: crypto first (smallest, drained
earliest), then random / shapes / arguments in a
follow-up session.

### Phase 3.6 — Stdlib drain into libs

**Goal**: shrink `default/*.loft` to genuine universal
stdlib by moving domain-specific code into libraries.
Surveyed 2026-05-24 — three drains worth doing alongside
the chunking work; everything else in `default/` stays.

**Stays in stdlib** (load order in `src/main.rs` after
this phase):

| File | What | Why it stays |
|---|---|---|
| `01_code.loft` | Operators, math, text basics, collections, panic/assert/log/print, parallel | Genuine stdlib — every program needs this |
| `02_files.loft` *(renamed from `02_images.loft`)* | File I/O + path helpers | Universal-enough; raising the floor to `use fs;` for every file-reading program is not a win |
| `03_text.loft` | Text utilities (`trim` / `replace` / `to_lowercase` / character class checks); minus `escape_html` + path helpers | Text manipulation is core |
| `04_stacktrace.loft` | `stack_trace()` | Debug primitive |
| `05_coroutine.loft` | `CoroutineStatus` + `exhausted()` | Language feature support |
| `06_json.loft` | `JsonValue` + `json_parse` + manipulators | Backs the `{x:j}` format specifier + `text as Foo` cast — JSON shape is the **language's default debug serialization** ([already shipped](#why-json-stays-stdlib--struct-json-is-already-built-in)), can't be opt-in |

**Drains** (each becomes a new lib / merges into existing):

| Source | Drained content | Destination | Chunk |
|---|---|---|---|
| `02_images.loft` | `Image`, `Pixel`, `FileResult` typed for images, PNG load/save (`png_load`, `png_save`), Image-specific helpers | Merge into existing `lib/imaging/` | `loft-libs-graphics` |
| `03_text.loft` | `escape_html` (sole HTML-domain helper) | New `lib/html/` | `loft-libs-net` |
| `03_text.loft` | Path helpers (`dir`, `basename`, `join`, `resolve`, `path_sep`) | Move to `02_files.loft` (co-locate paths with file I/O) | (stays in stdlib, just relocated) |

**File rename**: after the image-types drain, `02_images.loft`
becomes the home for File I/O + path manipulation.  Rename
to **`02_files.loft`** to match the new content.

**Why JSON stays stdlib — struct ↔ JSON is already built in**:

Verified 2026-05-24 by reading the parser + running a
round-trip test.  The serialize direction is the `:j`
format specifier (`src/parser/objects.rs:1155`,
`src/parser/mod.rs:256`, `src/codegen_runtime.rs:303` —
`OpFormatDatabase` walks the struct's type at runtime
with `json: true`).  The deserialize direction is the
`as <StructType>` cast on text (`src/parser/mod.rs:981-989`
routes through `OpCastVectorFromText`).  Both directions
are shipping behaviour:

```loft
struct Foo { a: integer, b: text }
f = Foo { a: 12, b: "hello" };
s = "{f:j}";          // serialize:   "{\"a\":12,\"b\":\"hello\"}"
g = s as Foo;          // deserialize: Foo { a: 12, b: "hello" }
// (and `{f:#j}` for pretty-printed output)
```

This makes JSON the language's default debug-print +
serialization format — `06_json.loft`'s `JsonValue` API
is the secondary tool for cases where the target type
isn't statically known.  Pulling JSON out of stdlib
would break the format specifier and the cast for every
program.

**Steps**:

1. Move Image / Pixel / Format type declarations from
   `02_images.loft` to `lib/imaging/src/imaging.loft`.
   The native PNG ops (`n_load_png` / `n_save_png`) are
   already declared in `lib/imaging/loft.toml::[native.functions]`
   per Phase 2 — just the loft-side type defs migrate.
2. Move `escape_html` from `03_text.loft` to a new
   `lib/html/src/html.loft`.  Pure-loft library, no
   `[native]` section.  Schedule for `loft-libs-net`
   chunk extraction.
3. Move path helpers (`dir` / `basename` / `join` /
   `resolve` / `path_sep`) from `03_text.loft` into
   `02_images.loft`'s leftover content.
4. Rename `default/02_images.loft` → `default/02_files.loft`.
5. Update `src/main.rs` stdlib load order (the filename
   change is mechanical; the load sequence stays).
6. Audit existing call sites — any internal use of the
   drained Image types / `escape_html` needs a `use
   imaging;` / `use html;` import.  Search:
   `git grep -E 'Image\b|Pixel\b|escape_html'`.
7. Update tests that exercise the drained surface —
   they need the new `use` lines.
8. Run `make ci`; expect failures rooted in missing
   `use` statements, fix until green.
9. Extraction-hygiene gate update:
   `tests/extraction_hygiene.rs::FORBIDDEN_LIBRARY_SYMBOLS_MANUAL`
   gains entries for any image-related native symbols
   that move from compiler-crate to `lib/imaging/native/`
   during the drain.

**Acceptance**:

- `default/02_files.loft` exists; `02_images.loft` does not.
- `default/03_text.loft` is smaller (no `escape_html`,
  no path helpers).
- `lib/imaging/` carries the Image / Pixel / Format type
  definitions.
- `lib/html/` exists with `escape_html`.
- `make ci` green.
- Total `default/*.loft` line count drops from ~2,500 to
  ~2,000 lines parsed at every startup.

**Why now, in the same sweep as chunking**: the drained
libraries (`lib/html/`, `lib/imaging/` post-merge) get
extracted in the same Phase 4-5 cycle that ships the rest
of the chunks.  Doing the drain afterwards would require
another monorepo edit pass.  Doing it before chunking
finalizes lets each chunk land with its final library
membership in one motion.

**Effort**: M (~1 day): the moves are mechanical but
call-site updates ripple.  Sequential after Phase 1c, in
parallel with Phase 3 / 3.5 / 4 prep.

### Phase 4 — Extract `loft-libs-core` (first chunk)

**Packages:** `arguments`, `random`, `crypto` (plus any
libraries produced by [Phase 3.6 stdlib drain](#phase-36--stdlib-drain-into-libs) —
typically `html`, optionally `fs` / `json` if drained).

**Why first:** small, stable, no graphics deps, no
inter-chunk deps.  Validates the per-chunk template before
larger chunks commit.  Includes the Tier-C drain target
(`crypto`) so phase 1's decoupling work gets immediate
real-world validation.  Also the **template-author** chunk
— produces the reusable artefacts that phases 5-7b copy.

**Note (2026-05-24):** `shapes` moved from this chunk to
`loft-libs-graphics`; its loft.toml comment is "Shape
drawing and 2D collision detection library" — a graphics
concern, not a core utility.

**Steps:** apply [§ Per-chunk extraction template](#per-chunk-extraction-template).

**Additional Phase-4-only deliverables** (resolve Open
Q #4 — "the reusable GitHub Actions workflow YAML, write
once, copy per chunk"):

- Author `.github/workflows/library-ci.yml` in
  `loft-libs-core`.  Runs: build loft → `loft test` over
  every package in the chunk → `loft --native test` → leak
  gate.  Reads chunk-local skip-lists.
- Define the skip-list format (likely `chunk-skips.toml`
  at the chunk-repo root: keys
  `interpreter_pkg_skip` / `native_pkg_skip` /
  `leak_allow`).
- Port `loft-libs-core`'s entries from the monorepo
  `LIB_PKGS_SKIP` / `LIB_PKGS_NATIVE_SKIP` /
  `SCRIPTS_LEAK_ALLOW` into that file.

**Acceptance:**
- `lib/{arguments,random,crypto}/` directories removed
  from the monorepo (plus any Phase-3.6 drain outputs
  if that phase completed first).
- `make ci` passes using registry-installed versions.
- `loft-libs-core` CI green on its own
  (interpreter + native + leak gates).
- `library-ci.yml` + `chunk-skips.toml` ship in
  `loft-libs-core` as the template referenced by phases
  5-7b.
- User code with `use random;` (etc.) sees identical
  behaviour to the in-monorepo version.

**Effort:** M (extraction itself) + S (workflow YAML
authoring — one-shot, amortised across all later chunks).

### Phase 5 — Extract `loft-libs-graphics`

**Packages:** `graphics`, `imaging`, `gridmesh`.

**Coordinates with** [`../02-graphics/`](../02-graphics/)
for API surface stability.  Mechanical once that plan's
API reaches a stable point.

**CI:** copy `library-ci.yml` + `chunk-skips.toml`
template from `loft-libs-core` (authored in Phase 4); port
graphics / imaging / gridmesh entries from monorepo skip
lists.

**Acceptance:** as phase 4 (chunk repo green on its own
interpreter + native + leak gates), plus moros-* consumers
(still in-monorepo at this point) updated to use the
external graphics chunk via their `loft.toml`.

**Effort:** M.

### Phase 6 — Extract `loft-libs-net`

**Packages:** `server`, `web`, `game_protocol`.

**Coordinates with** [`../08-server/`](../08-server/),
EVENT_LOOP, and multiplayer-editor plans for API stability
on `server` + `game_protocol`.  Includes the second
Tier-C drain target (`web`).

**CI:** copy `library-ci.yml` + `chunk-skips.toml` from
Phase 4; port net entries from monorepo skip lists.

**Acceptance:** as phase 4 (chunk repo green on its own
interpreter + native + leak gates).

**Effort:** M.

### Phase 7a — Split moros into shared world + moros-specific

Before moros extracts, the parts that aren't moros-specific
move into `lib/world/`.  This unblocks dryopea
([@PLAN46](../../plans/future/46-dryopea/README.md)),
keeps the existing TTT v5 + audience-generative-art
consumers of `lib/world/` working, and makes the eventual
`loft-moros` chunk genuinely moros-game-only.

**Why this split exists:** `lib/world/` today is a sparse
single-layer Cell/Chunk/World (TTT + audience).  The moros
side has a similar shape (hex grid, 32-wide chunks, floor-
division addressing — `lib/world/` already borrows the
addressing pattern from `moros_map`) plus walls, groups,
heights, and coupled geometry routines.  Dryopea uses the
same hex-world model AND the wall primitive directly:
dryopea lets the player issue build orders for walls (≥1
hex wide, walkable) and uses the same wall geometry for
rock faces in the terrain.  Three games sharing one library
is cheaper than three games re-implementing the geometry —
and the wall primitive in particular is load-bearing for
dryopea's signature gameplay (boss-breakable walls,
walkable battlements, terrain rock faces).

**Moves into `lib/world/`:**

| From | What | Notes |
|---|---|---|
| `lib/moros_map/types.loft` | Hex / Chunk types (the hex-grid layer addressing) | Palette + spawn types stay in `moros_map` (game-specific) |
| `lib/moros_map/moros_map.loft` | Chunk addressing helpers, hex math | Game-specific accessors stay |
| `lib/moros_sim/collide.loft` | Wall / hex geometry collision primitives | Game-specific tactical AI stays in `moros_sim` |
| `lib/wall.loft` (entire file) | Wall geometry constants (`DX`, `DY`, `DZ`, `STEP`) and the wall placement / hex-edge helpers | Folds directly into `lib/world/src/` (e.g. as `wall.loft`) — does NOT extract as a standalone `wall` package.  Both dryopea (user-built walls + rock faces) and moros use this; it's a core primitive of the world library, not an optional addition. |
| `lib/overland.loft` (entire file) | `OverlandMap` enum: per-hex material / item / height / water layers | Folds into `lib/world/src/` as a terrain-layer module.  Tiny (7 lines) but conceptually world-handling — moros + dryopea both want height + material per hex. |
| `lib/moros_*` | Group / height handling (per-hex layers, group adjacency) | Locate during execution; the consumer code is the ground truth for which pieces are game-agnostic |

**Stays in `lib/moros_*/`:**
- `palette.loft` (moros colour palette)
- `spawn.loft` (moros spawn-point semantics)
- Editor / UI / tools (`moros_editor`, `moros_ui`,
  `moros_sim/{editor,player,tools}.loft`)
- Renderer code that consumes the world model but is
  visually moros-specific (`moros_render`)

**Acceptance:**
- `lib/world/` exposes hex addressing, wall geometry,
  group / height handling — consumable by moros, dryopea,
  TTT v5, audience demo without re-implementation.
- `lib/world/` contains the folded-in wall primitives
  (`DX`, `DY`, `DZ`, `STEP`, placement / edge helpers) as a
  first-class API surface, not an internal detail.  Dryopea's
  build-order walls and rock-face terrain both consume the
  same primitive.
- `lib/wall.loft` removed from `lib/` root; its content
  lives under `lib/world/src/`.
- `lib/moros_*/` contains only moros-game-specific code;
  every `lib/moros_*/src/*.loft` `use world;` line resolves
  cleanly.
- Existing consumers (TTT v5 binary protocol, audience
  demo) continue to work — the existing sparse Cell/Chunk
  shape is preserved alongside the new hex-world additions
  (two cell models coexist in one package; they share
  addressing).
- All moros demos render identically before and after the
  split.
- The dryopea plan ([@PLAN46](../../plans/future/46-dryopea/README.md))
  can begin its consumer code against `lib/world/`,
  including user-built-wall placement and rock-face
  geometry on top of the wall primitive.

**Effort:** M (mechanical move + refactor of moros consumers).

### Phase 6w — Extract `loft-libs-world`

**Packages:** `world` (post-7a, expanded with hex
addressing, walls, groups, height, coupled geometry,
and the folded-in `wall.loft` content).

**Why after 7a:** `lib/world/` only has the full shared
surface AFTER 7a moves moros's spatial primitives in.
Extracting before 7a would publish an incomplete world
library that moros + dryopea couldn't actually use.

**Why before 7b:** the moros chunk depends on
`loft-libs-world` from the registry; world must be
published first.

**Numbering:** "6w" reflects that this chunk extraction
runs in parallel with phases 5 and 6 (chunk extractions
are independent once their prerequisites are met) — it
just additionally requires 7a (a monorepo-internal split).
Sequencing: ship 7a, then 6w can interleave with 5 / 6 in
any order.

**CI:** copy `library-ci.yml` + `chunk-skips.toml` from
Phase 4; port world's entries from monorepo skip lists
(if any — `world` is currently a small pure-loft package
with minimal native-codegen surface, so the skip list
should be near-empty).

**Acceptance:** as phase 4 (chunk repo green on its own
interpreter + native + leak gates), plus all four
consumers (moros monorepo, TTT v5 binary protocol,
audience-generative-art demo, dryopea plan-46 starter
code) successfully consume the registry-published version.

**Effort:** M.

### Phase 7b — Move moros packages into the existing `moros` project

**Packages:** `moros_editor`, `moros_map` (game-only
remnant after 7a), `moros_render`, `moros_sim`, `moros_ui`.

**Destination repo:** the **existing `moros` GitHub project**
(not a new `loft-moros` chunk repo).  Decision 2026-05-24:
the moros libraries colocate with the moros game in one
project — both for discoverability (users looking for "moros
stuff" hit one repo) and because the libraries are
moros-specific and co-evolve tightly with the game itself.

Layout inside the moros repo after this phase:

```
moros/
├── lib/
│   ├── moros_editor/
│   ├── moros_map/
│   ├── moros_render/
│   ├── moros_sim/
│   └── moros_ui/
├── src/                  # the moros game executable
├── loft.toml             # references registry deps: loft-libs-graphics, loft-libs-world
└── ...
```

Per-library registry tarballs publish independently — each
`lib/moros_*/` has its own `loft.toml` with its own
version.  `homepage` in each registry entry points at the
moros repo's `tree/main/lib/moros_*/` URL.

**Why after 7a:** the shared world primitives must be in
`lib/world/` and in `loft-libs-world` first; otherwise the
moros libraries drag world handling along with them and
dryopea can't reuse it.

**Why last overall:** mid-development; depends on
`loft-libs-graphics` AND `loft-libs-world` (registry
versions).

**CI:** copy `library-ci.yml` + `chunk-skips.toml` from
Phase 4 to the moros repo's `.github/workflows/`; port the
moros entries from monorepo skip lists.  This chunk has the
heaviest skip list (the moros packages are still mid-development
and surface the most native-codegen gaps — those entries follow
the code into the moros repo).

**Acceptance:** the moros repo's CI is green on its own
interpreter + native + leak gates; per-library tarballs
publish to the registry; the moros game still builds + runs
against the registry-installed libraries it depends on.

**Effort:** MH.

### Phase 7c — Dryopea project bootstrap (sibling to 7b)

When [@PLAN46](../../plans/future/46-dryopea/README.md)
starts execution, create a new **`dryopea` GitHub project**
following the same layout as `moros`: dryopea-specific
libraries colocated with the dryopea game in one repo, with
`loft-libs-world` (and any other shared chunks) consumed
from the registry.

This is not a chunk extraction (no `lib/dryopea_*/`
currently exists in the monorepo to extract).  It's a
greenfield project setup whose template is `moros`
post-Phase-7b.  Listed here for plan completeness; the
actual work lives in [@PLAN46](../../plans/future/46-dryopea/README.md).

**Effort:** S (bootstrap only; the dryopea-specific code
volume is whatever @PLAN46 ends up scoping).

### Phase 8 — Final monorepo cleanup

After all chunks extracted:

- `audience_crystal` stays in-monorepo (paired with the
  audience demo).  Add a package `tests/` directory so it
  joins the library CI gates (currently only covered by
  cross-mode equivalence tests).
- Decide fate of any unconverted single-file `lib/*.loft`
  modules: keep in-monorepo or fold into a sibling chunk.
- Update CLAUDE.md / lib_plans index entries that
  reference moved libraries.
- Update [`PACKAGES.md`](../../PACKAGES.md) to reflect
  monorepo-free state.

**Acceptance:** `lib/` directory holds only
`audience_crystal/` and any deliberately-retained
single-file modules.  No `lib/*/native/` source-code crate
references the monorepo `Cargo.toml` workspace.

**Effort:** S.

### Phase summary

| Phase | Scope | Depends on | Effort |
|---|---|---|---|
| 1 | Drain library symbols from `src/native.rs`; convert single-file modules | — | M (3 subs) — **DONE 2026-05-24** |
| 2 | Compile-time native-registry aggregator | Phase 1 | M — **DONE 2026-05-24** |
| 3 | PKG.REG + cdylib loader land | PACKAGES.md § Open work | — (external) — PKG.REG **code complete 2026-05-24**; ecosystem-bootstrap unblocked via interim `K_tmp` (no YubiKey needed for Phase 4 start) |
| **3.5a** | **Dry-run libraries WITHOUT monorepo consumers** (crypto) — move out, verify CI + reproducible package | Phases 1-2 only | S — **DONE 2026-05-24** (crypto → `../loft-crypto/`) |
| **3.5b** | **Implement real path-dep resolution** in `src/manifest.rs` + `src/parser/mod.rs` | 3.5a | S (~50 LoC + 1 unit test) |
| **3.5c** | **Dry-run libraries WITH monorepo consumers** (random, web, shapes, arguments) | 3.5b | S per library |
| **3.6** | **Stdlib drain** — Image types → `lib/imaging/`; `escape_html` → new `lib/html/`; path helpers `03_text.loft` → `02_files.loft`; rename `02_images.loft` → `02_files.loft` | Phase 1c | M (~1 day) |
| 4 | Extract `loft-libs-core` (arguments, random, crypto; plus Phase-3.6 outputs `html` etc.) — real publish through registry | Phases 1-3 + Phase 3.5 dry-run for the same package | M |
| 5 | Extract `loft-libs-graphics` (graphics, imaging, gridmesh, **shapes**) | Phase 4 + `../02-graphics/` | M |
| 6 | Extract `loft-libs-net` (server, web, game_protocol) | Phase 4 + `../08-server/` | M |
| 6w | Extract `loft-libs-world` (`world` expanded by 7a, wall folded in) | Phase 7a | M |
| 7a | **Split moros**: move shared world primitives (hex, walls, groups, height, geometry) into `lib/world/` | Phase 4 (monorepo-internal, no registry needed) | M |
| 7b | **Move moros libraries into the existing `moros` GitHub project** (game + libs colocated) | Phases 5 + 6w + 7a | MH |
| 7c | **Bootstrap `dryopea` GitHub project** (greenfield game + dryopea-specific libs, same model as moros) | @PLAN46 starts execution | S (bootstrap only) |
| 8 | Monorepo cleanup + audience_crystal hardening | Phase 7b | S |

The world split is monorepo-internal (Phase 7a) and can
land before the registry is ready — it produces no
user-visible change, only relocates files.  Phase 6w
(world chunk extraction) then happens once 7a is stable
and PKG.REG is live; it can interleave with 5 / 6 in
either order.

**Chunk-extraction phases (4 / 5 / 6 / 6w / 7b) are
serialised, not batched.**  Each one stands up a new
GitHub repo with its own CI, registry releases, and
issue tracker — bundle them together and the admin
work eclipses the actual code move.  Plan on at least
one minor release of soak between consecutive chunks
before opening the next.

**Each chunk phase has an internal sequencing too:**
the external chunk must be **finished, tested, and
published** (Stage A in the [per-chunk template](#per-chunk-extraction-template))
BEFORE the monorepo PR opens (Stage B).  And Stage B
itself is ordered: **link, validate the link, remove,
re-validate** — four separate commits in one PR, each
its own gate.  Never bundle "remove the old + link to
the new + hope CI proves the swap" into a single
operation — that hides regressions and makes rollback
expensive.

## CI path for libraries (built 2026-05-23) — travels with each chunk

The libraries now have a self-contained CI path in the monorepo that ports
unchanged to each extracted chunk (it keys off `lib/<pkg>/tests/*.loft` +
`loft.toml`, nothing monorepo-specific):

| Gate | Where | Covers |
|---|---|---|
| Interpreter | `tests/wrap.rs::library_suite` | every `lib/*/tests/*.loft` via `loft test` (subprocess, package-resolved); skips via `lib_test_skipped` (`LIB_PKGS_SKIP` / `LIB_TESTS_SKIP`) |
| Native | `tests/native.rs::native_library_suite` | the same via `loft --native test` (compiles each to native Rust, linking the package's `#native` crate); skips `LIB_PKGS_NATIVE_SKIP` / `LIB_TESTS_NATIVE_SKIP` (native-codegen gaps, [@P321](../../PROBLEMS.md)) |
| Leak | `tests/wrap.rs` `run_test` gate | unfreed stores at program exit fail; allowlist `SCRIPTS_LEAK_ALLOW` ([@P322](../../PROBLEMS.md)) |
| WASM | **Not yet implemented** — known gap, see [§ WASM gate](#wasm-gate--known-gap-before-chunk-extraction-starts) below | Should run `loft --native-wasm test` per package once the infrastructure exists |
| Quick dev loop | `make test-packages` | interpreter-only shell loop over every package test (dev-only, in `ci-full`; the cargo suites above are the gates) |

When a chunk extracts, its repo CI runs the equivalent (`loft test` +
`loft --native test` over the chunk's packages, plus WASM once it lands) and
the skip-lists travel with the code as the chunk's own `*_NATIVE_SKIP` /
`*_WASM_SKIP` / leak allowlist.  This is the "clear CI path for itself" the
libraries needed before living outside the monorepo (Open question #4 below).

### WASM gate — known gap before chunk extraction starts

`--native-wasm` is loft's third backend ([WASM.md](../../WASM.md)) and
PACKAGES.md's [target matrix](../../PACKAGES.md#target-matrix) lists
`wasm32-wasip2` as a first-class output.  Today the library CI gate
covers interpreter + native but NOT WASM — there is no `wasm_library_suite`
equivalent of `native_library_suite`.

**Why it matters for extraction:**

- A chunk repo that publishes a library used in browser-deployed loft
  games (graphics, world, moros — every game-adjacent chunk) silently
  ships a backend that nobody tested.
- The cdylib loader / `extensions.rs` design covers native dispatch;
  the WASM dispatch path uses prebuilt `.wasm` artifacts per
  PACKAGES.md `prebuilt/wasm32-wasip2/`.  Without a CI gate, those
  artifacts are unverified.

**Status — owned outside this plan:** a parallel effort (separate
Claude session, 2026-05-23+) is hardening the library CI infrastructure.
Adding a `wasm_library_suite` is the natural next step there and should
land BEFORE Phase 4 opens (the first chunk extraction).  Coordinate via:

- This plan blocks Phase 4 Stage-A5 ("chunk CI green") on WASM
  parity once the gate exists.
- The chunk-repo workflow YAML authored in Phase 4 must include a
  WASM job (or an explicit `skip-wasm` flag with rationale).
- Skip-list format gains `LIB_PKGS_WASM_SKIP` / `LIB_TESTS_WASM_SKIP`
  to travel with chunks.

**Prebuilt artifacts** (PACKAGES.md `prebuilt/<target>/`): out of scope
for the first extraction round.  Chunks build from source at install
time initially; prebuilt distribution is a PKG.REG follow-on once a
chunk has stable releases.

## Per-chunk extraction template

Each chunk extracts in **two stages with an explicit gate
between them**: the external chunk is built, tested, and
published on its own (Stage A) BEFORE any monorepo
linking begins (Stage B).  Then within Stage B the
monorepo swap is itself ordered: **link, validate the link,
remove, re-validate**.  Don't bundle these — each step is a
gate.

### Multi-package repo conventions (chunks + game projects)

When a single GitHub repo hosts multiple loft packages
(every chunk above, plus `moros` and `dryopea`), the
registry can't assume `loft package` runs at the repo
root.  Two conventions handle this:

**1. Per-package git tags** — `<package>-v<version>` (not
just `v<version>`).  Tags are repo-global, so naked
`v0.1.0` would collide when sibling libraries in the same
chunk both want to ship "0.1.0".  Per-package prefix keeps
the namespace clean and lets the registry's reproducible-
build re-check find the right snapshot:

| Chunk + package | Git tag |
|---|---|
| `loft-libs-core/crypto` v0.1.0 | `crypto-v0.1.0` |
| `loft-libs-graphics/imaging` v0.2.0 | `imaging-v0.2.0` |
| `moros/lib/moros_render` v0.3.1 | `moros_render-v0.3.1` |

**2. `subpath` field on registry version rows** — optional
JSON string pointing at the package directory inside the
repo, relative to the repo root.  Default `""` (single-
package repo) keeps existing single-library entries
unchanged; chunks set it to the library's subdir:

```json
"crypto": {
  "versions": {
    "0.1.0": {
      "url": "https://github.com/loft-lang/loft-libs-core/releases/download/crypto-v0.1.0/crypto-0.1.0.tar.gz",
      "subpath": "crypto",
      "sha256": "1c68ce3624…",
      "size": 5719,
      "loft": ">=0.8",
      "published": "2026-05-24T11:30:00Z"
    }
  }
}
```

Both fields land in
[PKG_REGISTRY.md § Schema](../../PKG_REGISTRY.md#schema)
when this plan executes.  `validate.py` honours `subpath`
when running the reproducible-build re-check: clones the
homepage at `<package>-v<version>`, `cd <subpath>/`,
`loft package`, compares the resulting sha256.

`SUBMITTING.md` gains a "publishing from a chunked repo"
sub-section showing the `cd subdir/` step + tag-prefix
convention for authors maintaining one of the chunks.

### Stage A — Build the external chunk

The chunk must be **finished and tested in isolation**
before we link to it.  No monorepo changes in this stage.

1. **Verify** every package in the chunk has `loft.toml`
   and passes its own tests in-tree against both gates:
   `tests/wrap.rs::library_suite` (interpreter) +
   `tests/native.rs::native_library_suite` (native).  Note
   the chunk's current `*_NATIVE_SKIP` and leak-allowlist
   entries — they travel with the code.
2. **Create the external GitHub repo** for the chunk
   (`loft-libs-core` / `loft-libs-graphics` / etc.).
3. **Drop in the CI workflow.**  Copy
   `.github/workflows/library-ci.yml` from `loft-libs-core`
   (the first chunk — Phase 4 authors it).  The workflow
   builds loft, runs `loft test` over every package in the
   chunk, then `loft --native test`, then the leak gate.
   Skip-lists travel as the chunk's own
   `chunk-skips.toml` (or whatever the workflow uses) —
   ported from the monorepo `*_NATIVE_SKIP` / leak-allowlist
   entries identified in step 1.
4. **Push library content** to the external repo,
   preserving git history via `git filter-repo` or
   `git subtree split` of the chunk's `lib/*` directories.
5. **Verify the chunk CI is green** on its own.  Same gates
   as the monorepo, just running standalone.  This is the
   **"finished and tested" gate** — do not advance to
   Stage B until this is true.
6. **Tag a v0.1.0 release** per package in the chunk
   (independent semver per package; the chunk repo holds
   them but they version independently).
7. **Publish to the package registry**: `cd <external-repo>
   && loft publish` (per-package).
8. **Smoke-test the published chunk from outside the
   monorepo.**  In a scratch directory: `loft install
   <pkg>` for each package in the chunk, then run a
   minimal consumer (`use <pkg>;` + one call) against
   each.  This confirms `loft install` works for a
   stranger, not just for the monorepo.

### Stage A → B gate

Before opening any monorepo PR, all of the following must
be true:

- The chunk repo's CI is green on its own.
- All packages in the chunk are published at v0.1.0.
- The Stage-A8 smoke-test resolved each package from the
  registry into a scratch directory without consulting
  monorepo paths.

If any of these is not true, do not start Stage B.

### Stage B — Switch the monorepo over to the registry

Open ONE monorepo PR with the steps below as **separate
commits** so each gate is independently reviewable and
revertable.  Do not collapse them.

B1. **Link.**  In a single commit, add `<X> = "0.1.0"`
    dependencies to every monorepo consumer's
    `loft.toml` for every package in the chunk
    (including other libraries not yet extracted).
    Update consumer `use` statements if the package name
    differs (typically no change).  Leave `lib/<X>/`
    directories in place.

B2. **Validate the link** (does NOT remove anything).
    Temporarily rename each `lib/<X>/` to
    `lib/_extracting_<X>/` (a name `probe_sibling_package`
    won't match) so the resolver MUST fall through to the
    registry-installed copy.  Run `make ci` against the
    registry version while the old tree is still
    physically present but path-invisible.  Green CI
    here = the registry version is functionally
    equivalent.  Commit the rename so the validating
    state is part of the PR history.

B3. **Remove.**  Delete the renamed
    `lib/_extracting_<X>/` directories.  Remove the
    chunk's entries from monorepo `LIB_PKGS_SKIP` /
    `LIB_PKGS_NATIVE_SKIP` / `SCRIPTS_LEAK_ALLOW` — those
    gates now live in the chunk repo.

B4. **Re-validate.**  Run `make ci` again.  Green here =
    nothing silently depended on the in-tree copy.  If a
    test fails because the registry version is missing
    behaviour the in-tree version had, **roll back B3**
    (the directory is still in git history) and file
    the gap as a chunk-repo issue.  Do not patch it in
    the monorepo.

B5. **Document** the extraction in
    [CHANGELOG.md](../../../../CHANGELOG.md) (user-facing)
    and link the chunk repo from CLAUDE.md's doc index
    if appropriate.

B6. **Finisher — update `tests/extraction_hygiene.rs`.**  Add
    every `n_*` symbol the chunk's libraries owned to
    `FORBIDDEN_LIBRARY_SYMBOLS` (with the owning
    `lib/<X>/native` path).  If the extraction removed a
    library-only dep from the main `Cargo.toml`, also add
    it to `FORBIDDEN_MAIN_CRATE_DEPS`.  The CI gate then
    locks the drain — a future PR that re-adds the symbol
    or dep to the compiler crate fails before merge.
    Same finisher applies to Phase 1 sub-tasks
    (1a / 1b / 1c) even though they don't go through
    Stage A/B (no external repo for an internal drain).

### Stage C — Ongoing maintenance

Subsequent updates land in the external repo; consumers
bump version in their `loft.toml`.  The monorepo no
longer carries the chunk's code — only its versioned
dependency.

## Open questions

These need decisions before the first extraction starts.
Listed here so future-you doesn't have to re-discover them.

1. **Naming convention.** `loft-<X>` (under loft-lang org)
   or `<X>` (org-namespaced)?  Affects `loft install` UX.
2. **Version policy.** Per-library independent semver, or
   monorepo-style coordinated bumps?  Independent semver
   matches package-registry idiom.
3. **Tagging.** RESOLVED (2026-05-24) —
   `<package>-v<version>` for multi-package repos (every
   chunk + moros + dryopea); `v<version>` only for the
   degenerate single-package case.  See
   [§ Multi-package repo conventions](#multi-package-repo-conventions-chunks--game-projects)
   for rationale (tag-collision avoidance when sibling
   libraries ship overlapping versions).
4. **Test infrastructure.** RESOLVED (2026-05-23) — the
   monorepo CI path (see [§ CI path for libraries](#ci-path-for-libraries-built-2026-05-23--travels-with-each-chunk))
   is the template: a chunk repo runs `loft test` (interp) +
   `loft --native test` (native) over its packages, carrying
   its own `*_NATIVE_SKIP` / leak allowlist.  The reusable
   GitHub Actions workflow YAML (`library-ci.yml`) + chunk-
   local skip-list format (`chunk-skips.toml`) is owned by
   [§ Phase 4](#phase-4--extract-loft-libs-core-first-chunk)
   as the template-author chunk; phases 5 / 6 / 6w / 7b
   copy it.
5. **Backwards-compatibility window.** When `lib/<X>/`
   leaves the monorepo, existing `use lib_<X>` (or whatever
   the current import shape is) should KEEP WORKING via
   the registry-installed copy for at least one release.
6. **Documentation home.** Per-library README.md migrates
   to the external repo.  CLAUDE.md doc index entry stays?
   Update to point at the external repo URL?
7. **Transitive deps.** If `lib/moros_render/` depends on
   `lib/graphics/`, does graphics extract first, or do they
   extract together?  Likely answer: stable libs extract
   first (graphics before moros_render), but moros_render's
   monorepo `loft.toml` updates to depend on the external
   graphics package as part of the graphics-extraction
   commit, not moros_render's later one.
8. **Cross-library breaking changes.** When an extracted
   library evolves and an in-monorepo consumer needs
   updates, who tracks the migration?
9. **World chunk vs graphics chunk.** `loft-libs-world` is
   proposed as its own chunk because three games consume
   it (moros, dryopea, audience demo) and dryopea blocks on
   it.  Alternative: fold `world` into `loft-libs-graphics`
   alongside `gridmesh` (also a geometry helper), keeping
   the chunk count at 4.  Trade-off: a separate world chunk
   is cleaner conceptually (no OpenGL coupling) but adds a
   fifth repo for users to find.  Resolve before phase 6w.
10. **Coexisting cell models in `lib/world/`.** Phase 7a
    keeps the existing sparse Cell/Chunk shape (used by TTT
    v5 + audience demo) alongside the new hex-world
    additions (moros + dryopea).  Two cell shapes that share
    addressing but differ in payload.  Decision: keep both
    as separate types in one package, or generalise (one
    parameterised cell type)?  Generalisation likely too
    invasive for 0.8.x — default is "keep separate, share
    addressing helpers".
11. **Destination for `logger.loft`.** ~~Decision pending~~
    **RESOLVED (2026-05-24) to option (c)** — leave as-is
    alongside the self-hosting cluster.  Consumer audit
    (`grep -rln 'use logger\b' --include='*.loft'`) returned
    a single hit: `lib/parser.loft`.  Parser stays in monorepo
    as self-hosting tooling, so promoting `logger.loft` to a
    standalone package or folding it into `lib/world/` would
    pay extraction cost (`loft.toml` authoring, registry slot,
    cross-package dep wiring) for a one-consumer scope that
    can move with parser if/when it grows a second consumer.
    The default-if-uncertain answer was (c); the audit didn't
    surface a reason to deviate.  Promotion remains an option
    later — if a non-parser consumer emerges (external library,
    `lib/world/` self-hosting cluster, etc.), revisit.
12. **Cross-chunk dep verification.**  See [§ Cross-chunk
    dependency graph](#cross-chunk-dependency-graph): two
    unknowns to confirm during Phase 1 — does post-7a
    `lib/world/` need `gridmesh` from
    `loft-libs-graphics`, and does `loft-moros` need
    `loft-libs-net` for multiplayer demos?  Answers
    determine chunk-publish ordering.
13. **WASM library CI gate.**  Owned by the parallel CI-
    hardening effort (see [§ WASM gate](#wasm-gate--known-gap-before-chunk-extraction-starts)).
    Must land before Phase 4 opens so the chunk-repo
    workflow YAML can include a WASM job.  Without it,
    extracted chunks ship an untested third backend.
14. **Pre-built artifact distribution** (PACKAGES.md
    `prebuilt/<target>/`).  Out of scope for first
    extraction round (chunks build from source at install
    time).  PKG.REG follow-on once chunks have stable
    releases.  Decision: which targets to prebuild
    (x86_64-linux, aarch64-macos, wasm32-wasip2 minimum),
    and who runs the matrix build (chunk-repo CI on tag,
    presumably).

## See also

- [PACKAGES.md § Open work](../../PACKAGES.md#open-work) — package registry +
  format infrastructure (PREREQUISITE)
- [`../../PACKAGES.md`](../../PACKAGES.md) — package
  format reference
- Sibling library plans whose libraries appear in the
  inventory above:
  - [`../02-graphics/`](../02-graphics/) — graphics library
  - [`../05-game-infra/`](../05-game-infra/) — game infra
  - [`../08-server/`](../08-server/) — server library
  - [`../10-game-client/`](../10-game-client/) — game client
- [`../../ROADMAP.md`](../../ROADMAP.md) — milestone
  placement (PKG.EXTRACT scheduled 1.1+)
