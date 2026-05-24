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

**ACTIVE 2026-05-23** — promoted from `future/`.  Trigger:
@P321c diagnosis revealed the project keeps re-adding library
code to the compiler crate (`src/codegen_runtime.rs` —
`n_sha256` via @P321a, the half-finished `n_load_png` /
`n_save_png` attempts during @P321c).  Activating this plan
reframes the goal: instead of adding MORE library code to
the compiler crate, **drain what's already there**.

Two prerequisites still gate the external-repo move:

1. **PKG.REG** (central registry MVP) landing in
   [PACKAGES.md § Open work](../../PACKAGES.md#open-work).
   Until `loft install <name>` works against a registry, there's
   no consumption path for an extracted library.
2. **Compiler-crate decoupling** — see
   [§ Prerequisite — decouple the compiler crate from library code](#prerequisite--decouple-the-compiler-crate-from-library-code)
   below.  Today the loft compiler still carries library Rust
   (`n_sha256` and 18 others in `src/native.rs`, plus crypto in
   `src/codegen_runtime.rs`), and the package manifest's
   `[native.functions]` table is consumed but the native registry
   is still hand-maintained.  Until the compiler crate carries
   zero library code, "extraction" is cosmetic.

The **prerequisite arc (Phase 1 — drain `src/native.rs` of
library symbols)** is unblocked NOW and is the first
actionable work under this plan.  PKG.REG is the gate on
the EXTERNAL-REPO move; the internal drain doesn't need it.

When unblocked: per-library extraction proceeds on its own
validated schedule.  Some libraries may extract early
(stable, low-churn); others stay in the monorepo until
their API matures.

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

| Chunk repo | Packages | Rationale |
|---|---|---|
| `loft-libs-core` | `arguments`, `random`, `crypto`, `shapes` | Small, stable, no graphics deps — extract first |
| `loft-libs-graphics` | `graphics`, `imaging`, `gridmesh` | Graphics stack + `#native` crates; coordinate with [`../02-graphics/`](../02-graphics/) |
| `loft-libs-net` | `server`, `web`, `game_protocol` | HTTP / multiplayer; coordinate with [`../08-server/`](../08-server/) |
| `loft-libs-world` | `world` (expanded by Phase 7a to absorb moros's shared spatial primitives: hex addressing, wall geometry, groups, height, coupled geometry).  Folds in `lib/wall.loft` content rather than carrying `wall` as a separate package. | Shared map / spatial primitives consumed by TTT v5, audience demo, moros, dryopea ([@PLAN46](../../plans/future/46-dryopea/README.md)).  Lives in its own chunk because consumers span multiple games and the dryopea plan blocks on it. |
| `loft-moros` | `moros_editor`, `moros_map` (game-only remnant after Phase 7a), `moros_render`, `moros_sim`, `moros_ui` | The moros RPG stack; depends on `loft-libs-graphics` AND `loft-libs-world`; extract last (mid-development) |

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
  ├── loft-libs-graphics   (shapes / arguments from core)
  │     ↑
  │     └── loft-libs-world   (shapes from core; gridmesh from graphics IF
  │           ↑                 world re-uses mesh primitives — verify in 7a)
  │           │
  ├── loft-libs-net        (crypto / arguments from core)
  │
  └── loft-moros           (depends on graphics + world; possibly net
                            for the multiplayer demos)
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
- Phase 7b (`loft-moros`) needs both `loft-libs-graphics`
  and `loft-libs-world` published.

**Open verification:** during Phase 1 (decoupling), each
chunk's actual `use` statements get audited and the graph
above gets confirmed or revised.  Notable unknowns:
- Does `lib/world/` (post-7a) need `gridmesh` from
  `loft-libs-graphics`, or are the geometries independent?
- Does `loft-moros` need `loft-libs-net` for `game_protocol`
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

Replace the hand-maintained static `NATIVE_TABLE` slice in
`src/native.rs` with `build.rs`-generated registration that
walks `lib/*/loft.toml::[native.functions]` and emits the
table at compile time.

**Acceptance:** adding or removing a library's
`[native.functions]` entry is the ONLY change required to
add/remove its symbols; `src/native.rs` contains zero
library entries.  Compiler crate carries no library code.

**Effort:** M.

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

What this DOES NOT yet do (Phase 2 step 2, still pending):

1. **`build.rs` codegen** — auto-generate
   `lib/<X>/native/src/lib.rs`'s `loft_register!` invocation
   from the manifest, eliminating that source of truth.
2. **Eliminate `#native "symbol"` annotations** in
   `lib/<X>/src/<name>.loft` — currently both the
   `#native` annotation AND the `[native.functions]` entry
   are present.  After step 2 lands, the annotation is
   redundant and can be removed.
3. **`src/native.rs::FUNCTIONS` (the stdlib NATIVE_TABLE)**
   stays hand-maintained — `[native.functions]` is for
   libraries, not stdlib.  See the stdlib-vs-library
   boundary table.

The metadata-driven groundwork is in place; the codegen
half can land independently when there's bandwidth.

### Phase 3 — Coordinate with PKG.REG (waiting phase)

Phases 4-7 cannot proceed until both:
- **PKG.REG** — registry MVP from [PACKAGES.md § Open
  work](../../PACKAGES.md#open-work).
- **cdylib loader** (`extensions.rs`) — also in PACKAGES.md
  § Open work; required so registry-installed packages
  contribute their `#native` symbols at install time.

No work owned by this plan.  Phases 1-2 are not blocked
on PKG.REG and should ship in parallel.

**Acceptance:** `loft install <name>` resolves a published
package; `#native` symbols dispatch through the cdylib
loader; `make ci` passes against a registry-installed copy
of one test package.

### Phase 4 — Extract `loft-libs-core` (first chunk)

**Packages:** `arguments`, `random`, `crypto`, `shapes`.

**Why first:** small, stable, no graphics deps, no
inter-chunk deps.  Validates the per-chunk template before
larger chunks commit.  Includes the Tier-C drain target
(`crypto`) so phase 1's decoupling work gets immediate
real-world validation.  Also the **template-author** chunk
— produces the reusable artefacts that phases 5-7b copy.

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
- `lib/{arguments,random,crypto,shapes}/` directories
  removed from the monorepo.
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

### Phase 7b — Extract `loft-moros` (moros-specific only)

**Packages:** `moros_editor`, `moros_map` (game-only
remnant after 7a), `moros_render`, `moros_sim`, `moros_ui`.

**Why after 7a:** the shared world primitives must be in
`lib/world/` and in `loft-libs-world` first; otherwise the
moros chunk drags world handling into the moros repo and
dryopea can't reuse it.

**Why last overall:** mid-development; depends on
`loft-libs-graphics` AND `loft-libs-world` (registry
versions).  Extracts as a unit since the moros-specific
packages co-evolve.

**CI:** copy `library-ci.yml` + `chunk-skips.toml` from
Phase 4; port the moros entries from monorepo skip lists.
This chunk has the heaviest skip list (the moros packages
are still mid-development and surface the most native-codegen
gaps — those entries follow the code into the moros repo).

**Acceptance:** as phase 4 (chunk repo green on its own
interpreter + native + leak gates), plus the moros demo
apps continue to build against the external moros chunk
(consuming `loft-libs-world` + `loft-libs-graphics` from
the registry).

**Effort:** MH.

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
| 1 | Drain library symbols from `src/native.rs`; convert single-file modules | — | M (3 subs) |
| 2 | Compile-time native-registry aggregator | Phase 1 | M |
| 3 | PKG.REG + cdylib loader land | PACKAGES.md § Open work | — (external) |
| 4 | Extract `loft-libs-core` (arguments, random, crypto, shapes) | Phases 1-3 | M |
| 5 | Extract `loft-libs-graphics` (graphics, imaging, gridmesh) | Phase 4 + `../02-graphics/` | M |
| 6 | Extract `loft-libs-net` (server, web, game_protocol) | Phase 4 + `../08-server/` | M |
| 6w | Extract `loft-libs-world` (`world` expanded by 7a, wall folded in) | Phase 7a | M |
| 7a | **Split moros**: move shared world primitives (hex, walls, groups, height, geometry) into `lib/world/` | Phase 4 (monorepo-internal, no registry needed) | M |
| 7b | Extract `loft-moros` (moros-specific only after 7a) | Phases 5 + 6w + 7a | MH |
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
3. **Tagging.** Are external repo tags `v0.1.0` style or
   `0.1.0` (matching loft.toml syntax)?  Rust uses `v`
   prefix; npm uses bare.
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
