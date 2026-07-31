
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

## Release cadence

Releases follow a **monthly rhythm**.  Each cycle has one long-lived
branch named for its **release month**, in `YYYY-MM` form (e.g.
`2026-07`).  All cross-theme work for the cycle lands on that branch,
and it ships at the **start of that month** — but only once the
language is **stable with a low bug count**.

A release is gated on **stability, not a fixed feature set**: if the
bug count is still high at the month boundary, the release slips and
the branch keeps stabilising.  When a cycle ships, the next month's
branch starts fresh from the new `main` tip (`2026-07` → `2026-08`).

What work is in scope during a cycle (the warm feature freeze that
began with the `2026-07` cycle) is described in
[ROADMAP.md § Feature freeze](ROADMAP.md#feature-freeze--heading-into-the-2026-07-cycle-added-2026-06-07).

**Cycle themes:**

- **`2026-07`** — stability, the package registry, and library hardening
  (extraction finished, registry maintenance + discovery, reproducible packaging).
- **`2026-08` — "become a better PHP"**: the server-side-web + database stack —
  the `#c` direct-C-ABI tier (@PLN24), the MariaDB/PostgreSQL clients (@PLN23),
  and the real HTTP server (@PLN4). Explicitly **not** `2026-07` work. Full
  rationale + critical path: [BROADENING.md § Better PHP](BROADENING.md#better-php--the-2026-08-cycle-theme).

### What forces a release — keep the list bounded

*Producing* a release is cheap — CI builds every target binary automatically — but every **category of
change that forces one** is a standing tax, so that list must stay **bounded**. An unbounded
release-coupled list is itself a contract-1 red flag: it means more and more work can only ship on the
monthly beat. What legitimately forces a loft release is exactly **a change to the loft binary**:
FFI / `#native` macro changes, opcode / semantics changes (e.g. the @PLN110 `len/size` flip),
performance fixes, and the occasional language feature. A tree-walker's behavior *is* its binary, so
these are inherently release-coupled and the set is naturally small.

**Everything else stays off the release axis** and ships on its own cadence — **libraries, the registry,
and docs are never release-coupled.** Coupling them would balloon the release-tied list and drag all
work to the monthly beat. The mechanism that keeps *libraries* off the axis is the resolver
dependency-gate (@PLN113 arc D): a library declares the loft version / contract it needs, the resolver
matches it, so a library update publishes independently and an older binary falls back — no coordinated
release. A binary-baked change libraries must adapt to (the flip) creates a **one-time** "libs need this
release to exist" dependency; after that release the libraries decouple again. Keep such couplings
one-time, never standing.

**Cadence preference: fewer releases, the monthly rhythm** — for people who want the latest performance
fixes and the occasional feature (not planned to be many, but not ruled out). Not a proliferation of
point releases; a point release off the monthly beat is for a genuine binary fix that cannot wait, not
a routine tool.

### Closing plans when the release merges

Plans live in [`loft-lang/plans`](https://github.com/loft-lang/plans); GitHub's
`Fixes #N` auto-close is **same-repo only**, so a loft PR can never auto-close a
plan.  Closing is explicit and cross-repo:

- **A PR that completes a plan** carries a close directive in its body —
  `Closes @PLN<n>` (or `Closes loft-lang/plans#<n>`).  The plan stays
  `status:active` while the work is only on the cycle branch.
- **On merge to `main`** (the release), the
  [`close-plans` workflow](../../.github/workflows/close-plans.yml) reads the
  merge PR's directives and runs
  [`scripts/close-shipped-plans.sh`](../../scripts/close-shipped-plans.sh) —
  setting each plan `status:finished` + closing it.  (Needs a `PLANS_TOKEN`
  secret: Issues:write on the plans repo; without it the job no-ops.)
- **Drift safety net (runs daily):** the nightly checks
  ([`miri.yml`](../../.github/workflows/miri.yml) → `stale-plans-audit`
  job) run `scripts/audit-stale-plans.sh` every day, warning when a
  `status:active` plan's close directive is already on `main` — so a missed
  close surfaces within a day, not at the next audit-by-hand (the drift this
  caught manually in `2026-06`: @PLN1/5/10/16/21).
- **Manual fallback:** run `scripts/close-shipped-plans.sh --range
  <prev-release>..main` once after the merge if the on-merge workflow didn't fire.

### `2026-06` — first monthly cycle, and the switch to calendar versioning

The monthly cadence is **adopted starting `2026-06`**, shipped **mid-month**
(2026-06-14) as a one-time exception to the "ships at the start of the month"
rule — the tree is stable and the library work is ready.  `2026-07` (branch
exists) then rebases onto the new `main` tip and resumes as the next cycle,
shipping at the start of July as normal.

This is also the **switch from semver `0.8.x` to calendar versioning**: the
release is named for its month (`2026-06`), which `Cargo.toml` spells
`2026.6.0` (year.month.patch — cargo needs three numeric parts with no leading
zeros).  Each month bumps the month digit (`2026.7.0`, …, `2026.12.0`,
`2027.1.0`); the patch slot is reserved for in-month security fixes
(`2026.6.1`).  Existing library `loft = ">=0.8"` constraints are still
satisfied by `2026.6.0`.

**Scope (frozen):** what is on `main` + the `../loft2` flat-namespace break +
bug fixes only — no other new features.

### `2026-07` — candidate release blockers

The gate is **stability + public face**, not features (see § Safety gate). As
of 2026-07-04 the filed-bug tracker is **empty** (`loft-lang/loft` has 0 open
issues; highest is #501, all closed), so the list below is the concrete
must-close set before the `2026-07` tag — grouped hard-blocker → public-face →
ecosystem → hygiene. The `2026-07` branch already exists; rebase it onto the
current `main` tip before working the list.

**A. Safety gate — hard blockers (a crash/leak slips the tag, no exceptions):**

- [x] **D-key-1 — parser crash on a value-position keyed slice.** ✅ FIXED
  2026-07-04. A keyed range slice (`x = idx[lo..hi]`) panicked at
  `src/variables/mod.rs:524` (`set_loop` with no active loop); a partial-key
  match (`x = idx[k1]` on a multi-key index) panicked later at codegen
  ("Iter should have been rewritten"). Both are a `for`-only iterator used in a
  value position — now a clean diagnostic. Fix: a save/restore `iterable_context`
  flag around the iterable parse in `parse_in_range`, `set_loop` tolerates the
  missing loop, and `parse_key` rejects both subscript forms in a value position.
  Guards: `tests/parse_errors.rs::keyed_{range_slice,partial_key}_in_value_position_is_error`
  + `tests/scripts/502-keyed-slice-for-only.loft` (the legit for/comprehension/
  exact-lookup paths, both backends). full parse_errors (157) + wrap (51) green.
- [x] **Re-verify the full safety suite on the tag candidate** — ✅ RUN
  2026-07-04 on the branch tip (D-key-1 landed). `cargo nextest run --release
  --no-fail-fast`: **2603 / 2606 passed, 182 skipped**; ownership fuzz gate
  2/2, `LOFT_POISON=1` leak suite 49/49, the D-key-1 area poison-clean on both
  backends. **No SIGSEGV, no signal crash, no compile/link failure, no `stores
  not freed`.** The 3 failures are all ENVIRONMENTAL, not safety-gate items and
  not regressions. ✅ `error_messages::baselines_are_locked_in` **FIXED**
  2026-07-04 — it was a network-dependent flake (a `use unknown_module`
  auto-install attempt prints a `[registry] … Dns Failed` line offline / a
  registry-not-found line online, neither in the golden); `normalise()` now strips
  `[registry]` lines, so the baseline is deterministic across CI / offline / the
  sandbox (suite green offline). Remaining 2 are browser-only: `html_wasm` /
  `html_asyncify` (need node + a headless browser — the brittle class
  §CI-hardening addresses); re-run on a browser CI runner to clear before tag.
  None block the safety gate.
- [x] **Zero-leak gate** — ✅ no `stores not freed` warnings across the suite
  run above (both backends); the `LOFT_POISON` leak suite is green.

**B. Public face — the website must not ship stale or broken (elevated 2026-07-04):**

- [ ] **🟢 BLOCKER — Brick Buster `--html` render: BOTH root causes FIXED; pending
  merge + rebuild.** Two layered bugs, both fixed:
  1. **Vector-arg host-import elision** (commit e8e13234, `src/generation/`) — the
     `--html` codegen dropped every `#native` call taking a `vector` (`gl_upload_vertices`
     / `gl_upload_canvas` / `gl_set_mat4`), so no buffers reached WebGL (blank canvas).
     Fixed → geometry draws. Guard `tests/html_gl_imports.rs` + `tools/wasm_imports.mjs`
     (browser-free import-table check).
  2. **Canvas mutations lost** (the "6-colour" residual) — root-caused with a direct GL
     texture probe: every `Canvas` draw method did `d = self.data; d[i]=…`, which C86
     H-Copy COPIES, so the whole software rasterizer silently drew nothing (the atlas /
     text textures were blank). The maker surfaced a formal contradiction (binding.md
     `B-HeapAlias` "heap aliases" vs heap.md `H-Copy` "heap copies"); a both-backends
     boundary matrix proved H-Copy, and that `&`-write-through was silently ignored for
     a vector lvalue. **Fixed by making `& vector` a writable alias** (commit 348a37f5 —
     a genuine language feature: `d = &self.data; d[i]=…` writes through, non-owning;
     plain bind still copies). binding.md corrected (`B-Copy`/`B-Ref-Alias`/`B-View`);
     guard `tests/scripts/503-vector-reference-alias.loft`. Graphics library adopts it in
     **PR loft-libs-graphics#10** (6 Canvas sites → `&self.data`); proven end-to-end (a
     red|blue `fill_rect` now reads back blue). Also fixed a pre-existing no-`main`
     test-runner crash surfaced en route (commit 0d967c62).
  **Remaining (merge-gated, no code left):** land the loft compiler changes → merge
  graphics PR#10 + republish graphics → rebuild `doc/brick-buster.html` (`make game`) in
  a graphics-enabled env → confirm the render gate passes (≥20 colours) → commit the fresh
  hero. Keep NOT committing a rebuilt hero until that gate is green. Original
  investigation below.
- [ ] ~~**🔴 render REGRESSED; hero cannot be freshly rebuilt.**~~ Investigated 2026-07-04 (network + headless chromium): the
  committed `doc/brick-buster.html` (last good build ~2026-06-13) renders
  correctly (headless render gate PASSES, 128 distinct colours), but **`make game`
  from current source produces a bundle whose canvas renders BLANK** — it clears
  to the background colour and draws NO sprites/content (render gate: `canvas.blank`,
  1 distinct colour, 6 s wait, SwiftShader on). Confirmed a real regression, not a
  timing/GPU artefact, by rendering old-vs-new side by side. **So the staleness was
  a symptom: the hero was frozen at the last working build because rebuilding breaks
  it.** **ROOT CAUSE CONFIRMED 2026-07-04 (NOT @PLN25 — predates it):** the
  `--html`/wasm codegen **drops every `#native` host-import CALL that takes a
  `vector` argument.** Proof — the wasm import table of the last-good build lists
  `loft_gl_upload_vertices` (vertex buffer, `vector<single>`), `loft_gl_upload_canvas`
  (atlas texture pixels, `vector<integer>`), and `loft_gl_set_mat4` (matrix uniform,
  `vector<float>`); a fresh build's import table is MISSING exactly those three,
  while every scalar-arg GL fn (`gl_draw`, `gl_bind_texture`, `gl_set_uniform_float`,
  …) survives. Runtime confirms it: instrumenting the fresh page shows `loft_gl_draw`
  fires (n=6) but `loft_gl_upload_vertices` is **never called** — so `ss_vao` is never
  populated and every `gl_draw(ss_vao, 6)` no-ops. No vertices, no atlas texture, no
  matrices ever reach WebGL → clear works, nothing draws. **The buffers aren't handled
  wrong — the buffer-upload calls are ELIDED.** Fix lives in the `--html` host-import
  codegen (`src/generation/` — the reachability / host-import declaration path,
  `reachable_functions` at `generation/mod.rs:345`): a host import with a `vector<T>`
  param must marshal it to the `(ptr, count)` the `loft-gl-wasm.js` side already
  expects (`new Float32Array(mem, ptr, count)`), not drop the call. **Do NOT commit a
  rebuilt hero until this is fixed** (I did not — kept the working committed build;
  broken rebuild saved at `/tmp/bb-fresh.html`). Once fixed, rebuild + the render gate
  must pass, then the freshness gate (§CI-hardening) keeps it honest. **Structural
  fix still applies: build+deploy the bundle from CI instead of committing it.**
- [ ] **Brick Buster visual gate — via the CPU Canvas atlas, not the browser.**
  Every sprite (ball / paddle / bricks / power-ups / particles) is drawn by
  `build_atlas()` → `graphics::Canvas` (`fill_rect`/`fill_circle`/`vline`/`hline`,
  no 3D — verified 2026-07-04): the **software rasterizer, zero GL**. So golden-test
  `build_atlas().save_png()` (fuzzy-MAE, headless, deterministic) — it exercises
  exactly the drawing primitives that regress (GL is stable; the primitives inside
  it break), at ~one PNG diff, no display. **Ready-made recipe (de-risked this
  session):** the harness already exists — `graphics`'s `native/tests/gold.rs`
  (`gold_compare(example, gold_name, max_abs, mean_abs)`, runs the example under
  `--interpret` so no GL/native-compile, decodes both PNGs, fuzzy-MAE compare,
  `UPDATE_GOLD=1` to capture). Add an `examples/25-brick-buster-atlas.loft` (copy
  `build_atlas()` verbatim + `at.save_png("25-brick-buster-atlas.png")`) and a
  `brick_buster_atlas_matches_gold()` case. **WHERE it must run:** the graphics
  library is *extracted* to `loft-libs-graphics` and its `graphics.loft` depends on
  the `math`/`mesh`/`scene` packages, so rendering needs a **working graphics
  install** — build it in the graphics library's own **library-CI** (or a monorepo
  job that `loft install graphics` first). It CANNOT run in an offline monorepo
  checkout (no registry ⇒ `use graphics` can't resolve its deps — confirmed
  2026-07-04). Keep one browser WebGL confirmation (`make test-html-render`) as a
  release-time check, but it is not the gating one.
- [ ] **Gallery** — regenerate and verify `doc/gallery.html` examples run
  (`scripts/build-gallery-examples.loft`, `gallery-examples.js`); GALLERY_CI
  green (both bundles instantiate — the `gallery` job's Node probe already gates
  js↔wasm consistency).

**CI hardening (this cycle — retires the recurring "broke the WASM bundle /
stale website" class; theme-aligned with `2026-07` library hardening):**

- [x] **The missing freshness gate — LANDED 2026-07-04.** The Node
  instantiate-probe (`tests/html_wasm.rs` for `--html`; the `gallery` job's
  "Probe committed wasm/js pair") already catches js↔wasm *consistency* but not
  *freshness*: a stale-but-consistent bundle passes (proven by the 12-day-stale
  `doc/brick-buster.html` being green). Rather than a rebuild-then-`git diff
  --exit-code` (which risks *flaking* on wasm build non-determinism — the exact
  brittleness that killed the last attempt), the gate is `scripts/check_bundle_fresh.sh`
  + an advisory `bundle-fresh` CI job: **git-only, deterministic** (no build), and
  **PR-scoped** — it fails only a change set that edits a bundle's source
  (`tools/brick-buster/25-brick-buster.loft`) without rebuilding the bundle in the
  same set, so it can never flake and never blocks an innocent PR. Tested on real
  history (flags the current staleness; passes a clean set; skips on an
  unresolvable base). Advisory now; **promote to a required check once item B
  rebuilds the currently-stale `doc/brick-buster.html`**. The gallery wasm bundle
  (whole compiler → wasm) stays covered by the job's rebuild + Node-instantiate,
  not this gate (its "source" is all of `src/`).
- [x] **Move the flaky browser render off the blocking PR path — LANDED
  2026-07-04.** The GPU/headless-browser-flaky binaries (`html_render` =
  Chrome + SwiftShader WebGL; `html_asyncify` = headless-page asyncify resume) are
  now excluded from the per-PR `test` run (a `pull_request`-only filter addition)
  and run in a new **nightly + push-to-main, Linux-only, retry-tolerant** step,
  mirroring the differential-oracle pattern already in the same job. The
  DETERMINISTIC node-based `html_wasm` instantiate-probe (the LinkError /
  import-mismatch catch) STAYS on the PR path — it does not flake. Net: the per-PR
  browser surface is now deterministic-only; the brittle pixel render is visible
  daily but never blocks a PR. *(The Canvas atlas golden that replaces its everyday
  value is the ready-to-build gate scoped in item B above.)*

**C. Library ecosystem — a coherent registry on day one:**

- [x] **Merge PR #18 + republish — DONE 2026-07-04.** PR #18
  (`lima-default-random-0.3.0`) merged to `loft-libs-core` main (commit 2a8aed5b,
  by jjstwerff). Registry republished via `registry_maintain.sh` (commit `bb191e7`
  on `loft-lang/registry`): `arguments` 0.1.3, `random` 0.3.0, `regex` 0.2.1,
  `cbor` 0.1.1, **and** `crypto` 0.3.5 (also stale) — 5 GitHub releases cut, each
  sha256-verified against its tarball, the index re-signed (trust-gate verified,
  the re-sign foot-gun avoided) and pushed. Coverage check now **0 findings across
  21 libraries** — the July registry is coherent.
- [x] **`loft install` smoke — VERIFIED out of band 2026-07-04.** Every check the
  install makes is confirmed against registry `main` directly (bypassing the stale
  raw-CDN): the index carries all 5 new versions (GitHub API), `loft-keygen verify`
  passes on the pushed `index.json` + `index.json.sig` ("signature valid"), and each
  tarball's sha256 matched its entry at publish. The literal `loft install
  <lib>@<version>` is gated only on raw-CDN propagation (~1h) — a time delay, not an
  action; re-run once `raw.githubusercontent.com` catches up (or point a fresh cache
  at the API index).

**D. Release hygiene:**

- [x] **CHANGELOG `2026-07`** — ✅ DRAFTED 2026-07-04 (review before tag). Leads
  with the **breaking type migration** (`text as int/float/single` → `τ?` fallible
  parse, with the `?? default` upgrade recipe), then the retired store-lifetime bug
  class, dense vectors, `&`-binding, the sandbox, browser/`--html`, and a fixes
  round-up incl. the four migrated libs. Verify the prose against the final scope
  before tagging.
- [ ] Bump `Cargo.toml` `2026.6.0` → `2026.7.0`; run the § Per-release ship
  checklist (tag, crates.io publish, per-OS binaries + stdlib checksums).

**Explicitly NOT `2026-07` blockers** (recorded so they are not re-litigated on
release day): open plans/features (@PLN86/87/88/90/91, …) — the release is
stability-gated, not feature-gated; the formal **D-op-1/2** operational
meta-deviations (differential-vs-definitional conformance, not bugs); the
**P54 auto-wrap diagnostic-drop** gap (medium diagnostic-quality, not a crash —
the two-stage `Struct.parse(json_parse(t))` form reports correctly); and the
other demo apps (server / game-client / scene) which ship on their own cadence.

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
- Crash bugs: none (B2-runtime, B3, B5, B7, P136, P142, P155 all closed)
- Zero-leak gate — wrap-suite `loft_suite` currently emits no
  `stores not freed` warnings across scripts 42/62/76/95; re-verify
  on the tag candidate
- Zero-ignore baseline approval — only the `regen_fill_rs`
  maintenance entry remains (candidate for permanent exemption)

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
| **Zero-leak gate** | H | `State::check_store_leaks` must emit no `Warning: N stores not freed at program exit` lines across the full test suite AND a hands-on run of every `tests/scripts/*.loft`.  As of 2026-04-21 the wrap suite's `loft_suite` produces no `stores not freed` warnings, and bare-interpret runs on the historically-flagged scripts (42, 62, 76, 95) are clean under `LOFT_STORES=warn` — the gate is currently green but must be re-verified on the tag candidate (including `LOFT_LOG=stores` on the parallel scripts, see below). | `src/state/mod.rs:1486` check_store_leaks |
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

Baseline as of 2026-04-21 — only one entry remains:
- `regen_fill_rs` → maintenance-only, not a test of runtime
  behaviour (regenerates `src/fill.rs`); candidate for
  explicit permanent exemption.

(B5/B7 ignores all removed once the underlying bugs were
confirmed closed; `file_content_nonexistent_trace` and
`sigsegv_repro_79_alone` no longer carry `#[ignore]` attrs.
`tests/wrap.rs::sigsegv_repro_79_alone` and the P136 skip of
`79-null-early-exit.loft` in `loft_suite` are also gone —
`cargo test --release --test wrap` reports 47 passed, 0 ignored.)

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
| A4 spatial<T> full implementation | After pre-gate added in 0.8.0 |
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
homepage    = "https://github.com/loft-lang/loft"  # ✓ done 2026-03-15
repository  = "https://github.com/loft-lang/loft"  # ✓ done 2026-03-15
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

### Tag & publish — the mechanics (draft-first, under immutable releases)

The org enforces **immutable releases**: a release's assets freeze the moment it
is published and cannot be added afterwards.  So the four platform bundles MUST be
attached while the release is still a **draft**.  The pipeline is built around this
ordering — the owner never publishes an empty release and then waits for binaries:

1. **Push the annotated tag** — `git tag -a vX.Y.Z -m "…" && git push origin vX.Y.Z`.
   The tag push (not a published release) is what triggers `release.yml`.
2. **Let CI build the draft.**  `release.yml` builds all four targets (linux-musl,
   macos-x64, macos-arm64, windows-msvc) and creates the GitHub release as a
   **draft** with every bundle + `.sha256` attached and notes generated.  If any
   build leg fails, no draft appears — investigate, don't ship a partial release.
   The draft job also attaches two derived assets: `loft-<v>-src.zip` (the source
   archive the registry entry names for the version itself) and
   `loft-<v>-registry-entry.json`.
3. **Review, then publish.**  Open the draft: confirm the four bundles are present
   (smoke-test each per step 10), edit the title/body if wanted, then click
   **Publish**.  Only this click freezes the release — by which point the binaries
   are already attached.  Publishing an existing-tag draft does not re-trigger the
   build.
4. **Submit the registry entry.**  Take `loft-<v>-registry-entry.json` from the
   published release, splice it into `loft-lang/registry`'s `index.json` under
   `packages.loft`, and re-sign (`scripts/registry-sign.sh`).  This is what makes
   the release reachable by `loft self-update`, and it is the *only* step that puts
   the binaries under a signature: the `.zip.sha256` sidecars travel over the same
   transport as the zips, so they catch a corrupted download, not a substituted one.
   The signed index is the root; everything below hangs off its hashes:

   ```
   index.json                        ← the ONE signature (Ed25519, 4 trust roots)
    ├ binaries[triple].sha256          → loft-<v>-<triple>.zip   checked once, at download
    │  └ manifest_sha256              → SHA256SUMS             checked any time, on what is INSTALLED
    │     └ bin/loft, default/*.loft, and every other file the bundle shipped
    └ version.sha256                   → loft-<v>-src.zip        the source the release was built from
   ```

   Do not hand-edit the hashes.  The entry is generated from the artifacts of the
   run that built them, so it cannot drift; retyping it reintroduces exactly the
   failure a signature cannot catch — an index that is correctly signed and names
   the wrong bytes.

**Forgetting step 4 is caught on the NEXT release, not by anyone noticing.**  Only
step 2 fails loudly; a missing registry entry just leaves `loft self-update`
reporting "no releases published to compare against" forever, which nobody is paged
by.  So the `previous release reached the registry` CI job goes red on the PR that
bumps `Cargo.toml`, unless the last release's entry is in the signed index with a
binary per published triple and a `manifest_sha256` on each
(`scripts/check-release-published.py`).  It gates that PR only — red on every PR
during the publish→merge window would just teach everyone to merge past it.  A
release with no `loft-<v>-src.zip` is exempt as predating the mechanism, derived
from the assets rather than a version constant someone has to maintain.

**Never** create-and-publish a release in one step (the pre-2026.7 flow):
publishing creates the tag and freezes the release before the binaries are built,
so immutable releases then reject the upload — v2026.7.0 shipped binary-less
exactly this way.

---

## Pre-Release Documentation Review

> **Load the `doc-quality` skill first** (`/doc-quality`) — and at the start of
> *any* documentation review, not just the release. It carries the comment/doc
> rules (legible-on-contact, serve-the-reader, matches-reality + stamp-vs-pointer)
> these steps apply; reviewing without it is how stale stamps and author-bookkeeping
> creep back in.

Run these steps before tagging a release.  **They are advisory, not blocking** —
only the [Safety gate](#safety-gate--blocks-every-release) (crashes / memory / leaks
/ test integrity) blocks a release.  A doc-quality finding must **never** hold a bug
fix that unblocks users.  The lints get their teeth elsewhere: a library earns the
registry **`verified`** mark only with clean lints, but it **releases and installs
regardless**.  Same rule as `lint_comments.sh` — advisory by design, never fails CI.

### 0 — User-visual documentation review (stdlib API + guides + comparison)

The clear, **advisory** review of everything a *programmer* reads: the stdlib API
reference, the guide pages, the comparison/perf pages, and the **flags & routines**
(the `make help` block and CLI flags).  It is built to neither
**gloss** (the tool visits every unit — page, example, symbol, claim — so nothing is
skimmed) nor be **diff-scoped** (every check runs over the WHOLE corpus, so a stale
remark from any past release surfaces now, not only what this release touched).  Run
it every release to *see* the state and fix what's cheap — it never blocks the tag.
Check definitions: [API_SURFACE.md § S7](API_SURFACE.md).

| # | Check | Command | Status |
|---|---|---|---|
| 0a | **Stdlib API surface** — no missing docs, no doc-quality (plan-tag/history) violations, no duplicate `pub fn`s | `scripts/api_lint.py --check default/*.loft` → `0 active` | **[now]** |
| 0b | **Guide-page code runs** — every example in `tests/docs/*.loft` executes on both backends (they are tests) | `make test` (the `docs` suite) | **[now]** |
| 0c | **No stale language in prose** — temporal/hedge words (`currently`, `planned`, `for now`, `not yet`, `TODO`, `Qn`) in guide + comparison prose; each removed or justified | `api_lint --check` over the doc corpus | **[build]** — fallback: `grep -rnEi '(currently\|planned\|for now\|not yet\|TODO\|coming soon)' tests/docs/*.loft doc/*.md` |
| 0d | **References resolve** — every `` `make <target>` `` / `--flag` named in prose is a real Makefile target / CLI flag; ([build]) every function/type/symbol too | `doc_review` (target+flag resolution) | **[now]** targets/flags · **[build]** API symbols |
| 0g | **Flags & routines** — the `make help` block is split into clear routine groups; CLI flags grouped; no oversized undivided block | `doc_review` (corpus E, sections) | **[now]** |
| 0e | **Capability & comparison claims** — negative claims ("no way to X") and the `00-vs-*`/`00-performance` tables rot when *other* code changes; reviewed via a per-page content-hash ratchet (re-surfaced only when the page changed, an example broke, a symbol vanished, or on a fixed every-N-release cadence) | doc-review baseline | **[build]** — fallback: manual review of `doc/00-vs-rust.html`, `doc/00-vs-python.html`, `doc/00-performance.html` + capability statements |
| 0f | **Regenerate + eyeball** — `gendoc` completes with no warnings; spot-check pages render | `cargo run --bin gendoc` | **[now]** |

**Why it won't gloss:** the unit is *page × (each example, each symbol, each listed
claim)* — the tool lists every one and a red item can't be skipped silently.
**First run flags everything** (empty baseline), forcing one complete pass over the
whole surface; thereafter the ratchet re-surfaces only what changed or is scheduled,
so coverage stays total without re-reading unchanged, still-valid prose.

**Current stdlib baseline (0a):** 36 findings (15 missing docs + 21 doc-quality),
tracked by the tool (`scripts/api_lint.py -c`) — a burn-down **goal**, not a release
precondition (loft's own findings never block loft's release).

### Deferred for pre-external-developer releases (2026-05-15)

Step 0's tooled checks (0a, 0b, 0f, and the auto parts of 0c/0d once built) run every
release as **advisory** signals — they surface silent-wrong content (e.g. "no way to
read raw bytes" while `byte_at` exists) regardless of external users, but like gendoc
they *inform* the release, they do not *block* it.  Only the subjective judgment in
0e and step 7 (topic flow) waits for external signal.

Until the project has regular external-developer interactions
that exercise the user-facing examples, **steps 5, 6, 7, and
the cross-platform smoke test below** are explicitly deferred.

Rationale: those steps validate the user-facing surface
(`.loft` examples, comparison pages, walkthrough topic flow,
fresh-install smoke).  Without external users hitting them,
the validation is closed-loop — the same author who wrote
the example reads it, sees nothing wrong, ships.  The
validation PAYS OFF once external users surface friction (a
stale example, a confusing topic order, a Windows symlink
issue); running it before that point is busywork that
delays the release without strengthening it.

**The author will do these manually** when they have the
feedback signal that makes them meaningful.  Until then:

  - Step 5 (user docs vs Unreleased changelog) — defer.
  - Step 6 (DEVELOPERS.md + comparison pages) — defer.
  - Step 7 (topic-flow ordering) — defer.
  - Cross-platform smoke test (Linux + macOS + Windows
    walkthrough run, VS Code extension install,
    example-open) — defer.

Steps 1-4 + 8 + 9 (internal-doc hygiene, broken-link
audit, clippy-suppression review, gendoc + PDF) are NOT
deferred — they protect the shipped artefact regardless of
external-user presence and stay as release gates.

The safety gate above (crashes / memory / leaks / test-suite
integrity) is also NOT deferred — it blocks every release,
external users or not.

**Lift this deferral** when external developers start filing
issues / opening PRs / asking documentation questions.  At
that point the validation steps gain real signal and become
worth running pre-tag.  Update this section when that
happens.

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

> The corpus-wide checks here are now the **step 0** gate (0a–0e).  This step
> remains the *changelog-driven* cross-check: that each shipped change is reflected.

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

### 10 — Per-OS binaries + stdlib checksums → registry

The registry ([PKG_REGISTRY.md](PKG_REGISTRY.md)) is the trusted distribution
point, so the toolchain itself ships through it — signed, with checksums users
can verify offline.

- **Build a release bundle per supported target.**  `release.yml` does this
  automatically on a tag push (see § "Tag & publish" above) via
  `scripts/make-release.sh`, building the four shipped triples:
  - `x86_64-unknown-linux-musl`
  - `x86_64-apple-darwin`, `aarch64-apple-darwin`
  - `x86_64-pc-windows-msvc`
  - (no `aarch64-unknown-linux-*` yet — add a matrix row when it is needed.)
- **Each bundle is a self-contained zip** — `bin/loft` + `default/` stdlib +
  examples + `loft-reference.pdf` + `SHA256SUMS` — attached to the **draft**
  release as `loft-<version>-<triple>.zip` (+ its `.zip.sha256`).
- **One manifest per bundle.**  `SHA256SUMS` covers every file it ships,
  `bin/loft` and each `default/*.loft` included, and is the authoritative list of
  what a bundle owns (`self_update::owned_files` reads the same file).  There is
  deliberately no second stdlib-only manifest: it described a subset of this one,
  which made two ways to validate a single installation.
- **Publish to the registry:** splice the generated entry into the signed
  `index.json` (`loft-lang/registry`) and re-sign per
  [REGISTRY_BOOTSTRAP.md](REGISTRY_BOOTSTRAP.md) / [REGISTRY_SUBMIT.md](REGISTRY_SUBMIT.md).
  Per target it carries the bundle URL + sha256 **and `manifest_sha256`** — the
  digest of that bundle's `SHA256SUMS`.  The zip's own hash is checkable exactly
  once, at download; the manifest digest is what lets `loft verify-self` re-check
  an INSTALLED tree against the signature at any time.
- **Verify:** on a clean host, `loft self-update` resolves a bundle, checks its
  hash against the signed index, and installs it; `loft verify-self` then reports
  "matches the release published in the signed registry index".
- **Verify on Windows specifically — the one case no test can cover.**  Run a real
  `loft self-update` on Windows, from the previous release to this one.  Replacing
  a *running* executable is the only genuinely platform-divergent step in the
  chain: `apply_bundle` renames the target aside and copies in, because a running
  binary cannot be overwritten there but can be renamed.  The unit tests exercise
  rename-then-copy on the daily Windows leg, but never against the `loft.exe` that
  is executing them, so this needs a published release and a Windows box.  Do it
  once per release, before announcing.

### Open work — reproducible builds (@PLN78 step 7)

`make-release.sh` emits `SHA256SUMS`, which is integrity, not a byte-identical
rebuild.  Everything above works without it; what it would upgrade is the
*meaning* of the published hash — from "this is the artifact the maintainer
uploaded" to "this is the artifact the source produces", which is the stronger
claim.  Deliberately off the critical path: it was sequenced last so it could
never block a user-visible installer, and closing @PLN78 does not make it urgent.
The registry already re-checks reproducibility for *libraries* (gate 3 clones the
tag and re-runs `loft package`); the toolchain is exempt because it is not a
`loft package`, so this is the gap that exemption leaves.

---

## Tooling prerequisites for release verification

These are the host-side tools used to verify a release before
tagging.  Install instructions live with each tool's upstream
docs (don't duplicate them here — they rot).  When a release
adds an item that needs a new tool, add the tool here.

| Tool | Used for | Install hint |
|---|---|---|
| Rust toolchain (`cargo`, `rustc`) | Build + test loft itself | https://rustup.rs |
| `cargo nextest` | CI-locally test runner (matches CI matrix) | `cargo install cargo-nextest` |
| VS Code | SH.1 grammar visual sanity + SH.2 extension verification | https://code.visualstudio.com |
| `vsce` | VS Code extension packager (`vsce package` for SH.2) | `npm install -g vsce` (needs Node 20+) |
| `gdb` | NDB.0 quality gate (Linux primary debugger) | OS package manager |
| `lldb` | NDB.0 quality gate (macOS primary, Linux alternative) | OS package manager / Xcode CLI tools |
| `objdump` | DWARF inspection for NDB.0 (`-h` lists debug sections) | OS package manager (GNU binutils) |
| `node` | JS-glue probes for browser quality gate; `vsce` runtime | https://nodejs.org (20.x+) |
| `python3` | JSON validation (`python3 -m json.tool`); generic scripting | OS package manager |
| `chromium` / `google-chrome` | WASM HTML build verification (already used by `make wasm-html-test`) | OS package manager |

### Cross-platform smoke test (per release)

Performed manually before each release tag, on each supported
platform:

- **Linux:** install loft from a fresh git clone, run any
  newly-shipped walkthrough top-to-bottom, install the VS Code
  extension, open an example.
- **macOS:** same.
- **Windows:** same — pay attention to symlink behaviour (the
  VS Code extension grammar symlink is the most likely point
  of failure on Windows).

### Per-release ship checklist (in addition to the safety gate above)

For each release, the relevant per-item plans hold their own
landing procedures (e.g. for 0.8.5: SH.1, SH.2, DX.1, DX.3 in
[`plans/36-developer-experience/`](plans/36-developer-experience);
NDB.0 in [`plans/34-native-debug/`](plans/34-native-debug)).
The cross-cutting work for ANY release is:

- [ ] All per-item landing procedures in the release's plans
      passed.
- [ ] `cargo fmt --all -- --check` clean.
- [ ] `cargo clippy --tests --release -- -D warnings` clean.
- [ ] Full local test suite green (`cargo nextest run --profile ci`).
- [ ] CI matrix on push (Windows / macOS / Linux) all green.
- [ ] Safety gate above (no new crashes, no leaks, no new H
      P-issues opened during the release window).
- [ ] Cross-platform smoke test done.
- [ ] Release artefacts produced (see § Release Artifacts
      Checklist below).
- [ ] `Cargo.toml` version bumped.
- [ ] `CHANGELOG.md` (user-facing) + `CHANGELOG_TECHNICAL.md`
      (contributor) entries written.
- [ ] Tag pushed; `release.yml` workflow runs.

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
