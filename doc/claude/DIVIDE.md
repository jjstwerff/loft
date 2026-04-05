# Library Repository Division Plan

This document plans the extraction of libraries from `loft/lib/` into
separate GitHub repositories for independent publishing and development.

---

## Current state

All libraries live under `lib/` in the main `loft` repository:

| Library | Type | Native crate | Dependencies |
|---|---|---|---|
| `arguments` | pure-loft | — | — |
| `crypto` | pure-loft | — | — |
| `game_protocol` | pure-loft | — | — |
| `shapes` | pure-loft | — | `graphics` |
| `random` | native | `loft-random` | — |
| `web` | native | `loft-web` | — |
| `imaging` | native | `loft-imaging` | — |
| `server` | native | `loft-server` | `web` |
| `graphics` | native | `loft-graphics-native` | — |

Standalone `.loft` files not yet packaged: `code.loft`, `docs.loft`,
`lexer.loft`, `parser.loft`, `logger.loft`, `wall.loft`, `testlib.loft`.

---

## Target: three repositories

### 1. `loft-graphics` — dedicated repo

Large, complex, platform-specific. Has its own Rust dependencies (glutin,
gl, winit, fontdue), 22 tutorial examples, and will grow into a full
graphics engine.

**Contents:**

```
loft-graphics/
  graphics/          # OpenGL bindings, canvas, color, font rendering
  shapes/            # Shape generation (depends on graphics)
  engine/            # (future) Scene graph, game loop, asset pipeline
```

**Rationale:** GPU/headless-GL CI requirements, high iteration rate during
engine development, different contributor profile (graphics programmers).

### 2. `loft-server` — dedicated repo

Complex networking stack with security-sensitive dependencies. Will grow
with TLS, ACME, auth, RBAC, and game-server features.

**Contents:**

```
loft-server/
  server/            # TCP, HTTP, WebSocket
  web/               # HTTP client (ureq) — server depends on this
  game_protocol/     # Multiplayer messaging protocol
```

**Rationale:** Security updates on networking crates need independent
release cadence. Integration testing requires network access. `web` is
bundled here because `server` depends on it and they share the HTTP domain.

### 3. `loft-libs` — monorepo for everything small

All remaining libraries: small Rust-crate wrappers and pure-loft utilities.
Easy to manage together, similar structure, low complexity.

**Contents (initial):**

```
loft-libs/
  random/            # RNG (rand_pcg wrapper)
  crypto/            # SHA-256, HMAC, base64
  imaging/           # PNG encode/decode (png crate wrapper)
  arguments/         # CLI argument parsing
```

**Future additions** (as they get packaged):

```
  json/              # JSON parse/serialize
  regex/             # Regular expressions
  csv/               # CSV reading/writing
  logger/            # Structured logging
  ...
```

---

## What stays in `loft`

- `default/*.loft` — standard library, tightly coupled to interpreter version
- `loft-ffi/` — the FFI helper crate, used by all native libraries
- `tests/lib/` — test packages for the library loading mechanism itself
- Standalone `.loft` files (`lexer.loft`, `parser.loft`, etc.) — these are
  tools for the language itself, not user-facing libraries

---

## Migration steps

### Phase 1: Prepare (before any move)

- [ ] **P1.1** Ensure all library tests pass: `make test`
- [ ] **P1.2** Tag the current state: `git tag pre-lib-split`
- [ ] **P1.3** Create the three GitHub repositories:
      `loft-graphics`, `loft-server`, `loft-libs`
- [ ] **P1.4** Design a shared CI workflow template for library repos
      (build native crates, run `loft` test discovery on `tests/`)
- [ ] **P1.5** Decide on `loft-ffi` distribution: publish to crates.io
      or use git dependency. Native libraries in external repos need to
      reference it somehow

### Phase 2: Extract `loft-graphics`

- [ ] **P2.1** Create `loft-graphics` repo with README, LICENSE, CI
- [ ] **P2.2** Copy `lib/graphics/` and `lib/shapes/` preserving directory
      structure. Update `shapes/loft.toml` dependency path
- [ ] **P2.3** Copy or symlink `loft-ffi` (or point Cargo.toml at crates.io /
      git dep)
- [ ] **P2.4** Verify: all graphics and shapes tests pass standalone
- [ ] **P2.5** Set up release CI: on tag push, build zips per library,
      attach to GitHub Release
- [ ] **P2.6** Remove `lib/graphics/` and `lib/shapes/` from main repo

### Phase 3: Extract `loft-server`

- [ ] **P3.1** Create `loft-server` repo with README, LICENSE, CI
- [ ] **P3.2** Copy `lib/server/`, `lib/web/`, `lib/game_protocol/`
- [ ] **P3.3** Update `server/loft.toml` dependency on `web` to use local
      path within the new repo
- [ ] **P3.4** Verify: all server, web, and game_protocol tests pass
- [ ] **P3.5** Set up release CI
- [ ] **P3.6** Remove from main repo

### Phase 4: Extract `loft-libs`

- [ ] **P4.1** Create `loft-libs` repo with README, LICENSE, CI
- [ ] **P4.2** Copy `lib/random/`, `lib/crypto/`, `lib/imaging/`,
      `lib/arguments/`
- [ ] **P4.3** Verify: all tests pass
- [ ] **P4.4** Set up release CI (single release, one zip per library)
- [ ] **P4.5** Remove from main repo

### Phase 5: Clean up main repo

- [ ] **P5.1** Remove empty `lib/` directory (or keep only for `loft-ffi`)
- [ ] **P5.2** Update documentation: CLAUDE.md, EXTERNAL_LIBS.md, PACKAGES.md
      to reference the new repos
- [ ] **P5.3** Update `LOFT_LIB` / `--lib` documentation to explain how
      users point at externally cloned libraries
- [ ] **P5.4** Consider a `loft install` command or script that clones/
      downloads libraries from their repos

---

## Release workflow (per library repo)

Each repo publishes releases with one zip per library:

```yaml
# .github/workflows/release.yml
on:
  push:
    tags: ['v*']
jobs:
  release:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Package libraries
        run: |
          for dir in */; do
            [ -f "$dir/loft.toml" ] || continue
            name="${dir%/}"
            zip -r "${name}-${GITHUB_REF_NAME}.zip" "$dir"
          done
      - uses: softprops/action-gh-release@v2
        with:
          files: "*.zip"
```

Download URLs follow the pattern:
```
https://github.com/<org>/<repo>/releases/download/v1.0.0/<library>-v1.0.0.zip
```

These URLs map directly into the loft package registry format described in
REGISTRY.md.

---

## Open questions

1. **`loft-ffi` distribution** — crates.io publish vs git dependency?
   Publishing to crates.io is cleaner for external library authors but
   adds a release step. Git dependency is simpler for now.

2. **Shared versioning vs per-library versioning** — within each repo,
   do all libraries share a version (simpler) or version independently
   (more flexible)? Recommend starting with shared versions.

3. **CI loft binary** — library repos need a `loft` binary to run tests.
   Options: download from GitHub Releases, build from source as CI step,
   or use a pre-built Docker image.

4. **Transitive dependencies across repos** — `shapes` depends on
   `graphics` (same repo, fine). If a future `loft-libs` library needs
   `server`, that's a cross-repo dependency. The registry / `loft install`
   needs to handle this.
