<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# lib-plan 30 — loft binary distribution + self-update + advisory integration

**Status:** FUTURE (slot created 2026-05-31).  **The concrete implementation
design is [design.md](design.md) (2026-07-31)** — it supersedes the phase sizing
below, which predates the package trust chain: most of what these phases propose
to build has since been built for packages, and 30.5 is already done.  The threat
model and trust-chain analysis here still stand.

Filed because the
@PLAN12 § Phase 6.7 security-advisory channel needs a signed,
installable binary distribution to act on yank signals against the
loft binary itself — `loft self-update` to a fixed version is the
mechanical fix path that an advisory like *"loft 0.8.4 yanked,
fix in 0.8.5"* invokes.  Without this slot, 6.7 surfaces the
problem for the binary but has no companion fix mechanism.

Distinct from PACKAGES.md § Open work and the existing
PKG_REGISTRY.md design — those cover **library** distribution
(`loft install <pkg>`).  This slot is **the toolchain itself**
(`loft self-update`, `curl | sh` installer, OS-package
maintainership, signed per-platform binaries).

## Why a separate plan-slot, not a sub-phase of @PLAN12

@PLAN12 is the library-extraction arc.  Binary distribution is
its own multi-release effort:
- Signing-key operations (the existing Ed25519 root used for the
  library registry signs binary tarballs too; key rotation is a
  shared concern).
- Multi-platform CI pipeline (Linux x86_64 / arm64, macOS
  x86_64 / arm64, Windows x86_64, plus possibly the wasm32
  toolchain).
- OS package maintainership (Homebrew tap, apt repository,
  Chocolatey, maybe AUR — each has its own maintainer dynamics).
- Release cadence + LTS policy.
- Self-update UX + bootstrap-installer security audit surface.

Bundling this into @PLAN12 would distort that plan's
"library-extraction" framing.  Promoting to its own slot keeps
each plan focused.

## Goal

A loft user can:

```
curl -sSL https://loft-lang.org/install.sh | sh
loft --version          # works on first install
loft self-update        # later, when an advisory tells them to
loft self-update 0.8.5  # pin a specific version
```

With every artifact (the installer, every per-platform tarball)
signed by the same Ed25519 root key used by the library registry,
recorded in `index.json` with a sha256 + size, and yankable via the
6.7 `advisories.json` channel.

## Trust chain (bootstrap analysis)

The chicken-and-egg: to run `loft install loft@0.8.5`, the user
needs loft.  Resolved the same way Rust does it with `rustup`:

```
┌────────────────────────────────────────────────────────────┐
│ User trusts:                                               │
│   • install.sh hosted at loft-lang.org (HTTPS + cert)      │
│   • OR the Ed25519 public key fingerprint published in     │
│     multiple channels (release notes, docs, GitHub repo)   │
└────────────────────────────────────────────────────────────┘
              │
              ▼
┌────────────────────────────────────────────────────────────┐
│ install.sh has the Ed25519 public key embedded             │
│   • Downloads loft-<platform>-<version>.tar.gz             │
│   • Downloads loft-<platform>-<version>.tar.gz.sig         │
│   • Verifies signature against embedded key                │
│   • Extracts to ~/.loft/bin/loft                           │
└────────────────────────────────────────────────────────────┘
              │
              ▼
┌────────────────────────────────────────────────────────────┐
│ Installed loft has the SAME key baked in                   │
│   • `loft self-update` fetches index.json, picks newest    │
│     active version for this platform, verifies sig         │
│   • `loft audit` checks installed version against          │
│     advisories.json (6.7)                                  │
└────────────────────────────────────────────────────────────┘
```

The only outside-the-system trust is the install.sh download.
After that, every action is signed against the embedded key.
Match `rustup`'s model exactly.

## Goal — security signal closure

With 30 + @PLAN12 6.7 both shipped, the loop closes:

```
1. CVE filed against loft 0.8.4 (parser bug, stdlib bug, or runtime bug).
2. Maintainer publishes loft 0.8.5 with fix → registry entry.
3. Maintainer publishes advisory entry in advisories.json:
     "package": "loft", "affected": "<0.8.5", "severity": "security_critical"
4. User runs `loft script.loft` next time:
     [6.7] advisory check fires → "loft 0.8.4 yanked, fix in 0.8.5"
5. User runs `loft self-update`:
     [30] self-update fetches signed 0.8.5 tarball → atomic replace.
6. Next invocation runs clean.
```

The mechanical chain is what makes a registry-style yank
meaningful for the toolchain.  Without 30, step 5 is "build from
source — sorry."

## Phase outline

Five phases.  Numbered to suggest sequencing, but 30.2/30.3
can run in parallel.

### Phase 30.1 — Reproducible-build pipeline

**Goal:** byte-identical binary builds from the same source on
the same compiler version.  Required so the registry can verify
signed tarballs were built from the claimed source tree (mirrors
the library gate-3 reproducible-build re-check that ships
2026-05-31 for libraries).

Tooling (existing in the Rust ecosystem):
- `cargo-dist` or `cross` for the build matrix.
- `--remap-path-prefix` to strip absolute paths from debug info.
- `SOURCE_DATE_EPOCH=0` for embedded timestamps.
- `RUSTFLAGS="-C strip=debuginfo"` for slim binaries (or ship
  debuginfo as a separate sidecar).
- Sorted archive entry order + zero mtime in tarball headers
  (same fix the `loft package` shipped 2026-05-31 for libraries
  — reuse the determinism logic).
- Pinned Cargo.lock + pinned rustc-version.  The rustc version
  is part of the trust input — record it in the release notes
  and the registry entry.

**Done when:** two independent builders running the same source
+ same rustc + same flags produce byte-identical tarballs.
Gate-3-equivalent verifier in registry CI re-runs the build and
matches sha256.

### Phase 30.2 — Signing + registry entries for binaries

**Goal:** every released binary tarball lands in `index.json`
with a sha256, size, and Ed25519 signature, alongside library
packages.  Schema:

```json
"loft": {
  "kind": "toolchain",          // distinguishes from library packages
  "homepage": "https://github.com/loft-lang/loft",
  "versions": {
    "0.8.5": {
      "platforms": {
        "linux-x86_64": {
          "url": "https://github.com/loft-lang/loft/releases/download/v0.8.5/loft-linux-x86_64-0.8.5.tar.gz",
          "sha256": "...",
          "size": ...,
          "rustc": "1.83.0"
        },
        "linux-arm64":   {...},
        "macos-x86_64":  {...},
        "macos-arm64":   {...},
        "windows-x86_64":{...}
      },
      "published": "2026-..."
    }
  }
}
```

The signing key is the same Ed25519 root that signs `index.json`
itself.  Per-tarball `.sig` files sit alongside the tarballs in
the GitHub release assets.  Registry PR validator gate-2
(tarball sha256 + size) verifies for binary entries the same
way as for libraries.

### Phase 30.3 — `install.sh` installer

**Goal:** small, auditable shell script (`<100` lines) that
detects the user's platform, fetches the right binary,
verifies the signature, installs to `~/.loft/bin/`.

Components:
- Platform detection (`uname -s`, `uname -m`).
- Fetch `index.json` (or a smaller `latest.json` that just maps
  platform → latest stable URL, regenerated by CI on each
  release).
- Download tarball + signature.
- Embedded Ed25519 public key as a base64 constant in the
  script.
- Verify via `openssl` / `signify` / `minisign` (whichever is
  most portable — likely `minisign` since it's already in
  package managers and matches the Ed25519 model).
- Extract + atomic install (rename trick).
- Suggest adding `~/.loft/bin` to PATH.

**Done when:** a fresh Linux/macOS shell with `bash` and
`curl` can install loft via one `curl | sh` invocation; the
installer fails-closed on signature mismatch.

### Phase 30.4 — `loft self-update` + binary self-check

**Goal:** the installed loft binary updates itself AND verifies
its own integrity on every invocation (the binary's analogue of
@PLAN12 § Phase 6.7 verification moment 2 — per-invocation
library hash check).

**Verification timing for the binary** (mirrors and extends
6.7's library-side timing table — and uses the SAME
verify-on-recompile model to keep steady-state cost near zero
for the "many small runs of small scripts" use case loft is
optimised for):

| # | Moment | What's verified | Default | Off-switch |
|---|---|---|---|---|
| 1 | **Install / self-update** | sha256 (matches `index.json` toolchain entry) + Ed25519 sig + sanity `--version` invocation on the downloaded artifact | always | — |
| 2′ | **Startup mtime check** | `stat` the running binary; compare mtime to recorded value in `~/.loft/installed.toml`.  Match → skip hash, proceed.  Drift → hash binary, compare to recorded sha256, refuse on mismatch. | on | `LOFT_NO_SELF_VERIFY=1` |
| 3 | **Advisory check** | `(loft, version)` tuple checked against cached `advisories.json` (6.7's feed); fail/warn per severity for the binary the same way as for libraries | on | `LOFT_OFFLINE=1` only suppresses the feed refresh, not the check itself |
| 4 | **Embedded-key check on self-update** | downloaded binary's Ed25519 public key (extracted from a known offset) matches expected key → catches an attacker that swapped the binary AND its sig with a binary signed by a different (attacker-controlled) key | always | — |
| 5 | **`loft audit`** | re-hash the running binary (ignoring any cached mtime sentinel); re-check tuple against advisories; report drift | manual | — |

**Steady-state cost** (no binary tamper, no self-update):
just moment 2′'s `stat` (~1µs) and moment 3's in-memory
advisory lookup (~µs).  Effectively free.

**When the hash actually fires** (moment 2′'s drift branch):

- After a successful `loft self-update` — installed.toml gets
  rewritten with the new (mtime, sha256); next startup sees
  mtime match, no hash.  But the install step itself already
  hashed (moment 1).
- After an OS package update (Homebrew, apt) replaces the
  binary — mtime drifts → next startup hashes → mismatch
  surfaces in stderr "binary integrity check failed; was this
  installed via `loft self-update`?  Re-run installation."
  This is intentional: OS-package-manager installs need their
  own trust chain.  Users on apt should hash via apt's
  signature, not loft's.
- After a tamper that modifies the binary's bytes — drift
  detected, hash fires, mismatch caught.
- After a tamper that modifies the bytes AND restores the
  mtime — moment 2′ misses; caught by `loft audit` (moment 5)
  or by next `loft self-update` cycle when installed.toml
  rewrites.

**Threat model honesty.**  The retired moment "hash binary on
every startup" was paying ~30-50ms per invocation (or ~1ms
with markers) to catch the modify-and-restore-mtime attack
— a specific tamper that requires write access to the loft
binary's install path.  At that access level, the attacker
could equally swap installed.toml itself, set
`LOFT_NO_SELF_VERIFY=1` in the user's shell rc, replace the
shell entirely, etc.  The verify-on-drift model gives
99% of the security with effectively zero ongoing cost; the
remaining 1% (modify + mtime restore) is the explicit
`loft audit` escape hatch's job.

Stdlib coverage: `default/01_code.loft`, `default/02_files.loft`,
`default/03_text.loft` are baked into the binary via
`include_str!` (per @PLAN12 § Phase 3.6 — drain doesn't shrink
to zero; the non-drainable floor stays embedded).  So moment 2
covers the embedded stdlib by transitivity — no separate
verification path needed for those files.

**Self-update flow:**

1. Load cached `index.json` (refresh if stale per @PLAN12 6.6
   TTL rule).
2. Find the newest active version for the current platform.
3. If user passed an explicit `<version>` arg, use that.
4. Download tarball + signature to a temp file.
5. Verify signature against the public key embedded in the
   running binary (rotation-handled by Phase 30.5).
6. Verify the new binary's embedded public key matches the
   expected key (moment 4) — catches a swap of "binary +
   matching-key" pairs.
7. Sanity-check the downloaded binary by running `loft --version`
   on it (still moment 1 — proves the binary at least executes).
8. Atomic replace.  Unix: rename to target.  Windows: rename
   existing → `.old` first, then move new in.
9. Write the new (version, sha256) entry to
   `~/.loft/installed.toml` (consumed by moment 2 on subsequent
   startups).
10. Print "loft 0.8.4 → 0.8.5; release notes: <url>".

**Startup self-check flow** (every `loft` invocation —
moment 2′):

1. Stat the running binary path.
2. Read `~/.loft/installed.toml`.  If absent (e.g.
   user built from source rather than installed via
   `loft self-update`) → skip self-check, proceed; no
   warning (build-from-source is a legitimate trust path).
3. If installed.toml's recorded mtime matches the running
   binary's mtime → skip hash, proceed.  Cost: one `stat`,
   microseconds.
4. Else (drift detected) hash the binary's bytes; compare
   to installed.toml's recorded sha256.
5. Match → rewrite installed.toml with the new mtime
   (keeps subsequent invocations on the cheap path); proceed.
6. Mismatch → refuse to start; print "binary integrity
   check failed: expected <sha> got <sha>; ran
   `loft self-update` or replaced via OS package manager?
   Re-install or run `loft self-update` to restore the
   trust record."
7. `LOFT_NO_SELF_VERIFY=1` skips steps 1-6 with a one-line
   stderr note `[verify] self-verify disabled
   (LOFT_NO_SELF_VERIFY)`.

The mtime-as-cache-key approach mirrors @PLAN12 § Phase 6.7
moment 2′ (compile-time library hash + mtime invalidation).
Both the binary and the libraries follow the same shape:
trust-on-install, re-verify only when something changed.

**Done when:** `loft self-update` on Linux/macOS/Windows
correctly upgrades a running install with no shell state
lost; sig mismatch → no replacement; downgrade requires an
explicit version arg (no implicit rollback); startup
self-check fires on every invocation and refuses to start on
detected tamper; warm-cache self-check is <1ms via the marker.

### Phase 30.5 — Key rotation + LTS

**Goal:** the embedded public key in installed binaries can be
rotated when the root key changes (planned `K_tmp` → YubiKey
`K_real` rotation in PKG_REGISTRY.md / REGISTRY_BOOTSTRAP.md).

Mechanism:
- New releases carry the new public key embedded in their
  binary.
- During the rotation window, the registry signs `index.json`
  with BOTH the old and new keys (`index.json.sig` becomes
  `index.json.sig.k1` + `index.json.sig.k2`, both checked).
- Installed binaries trust both during the transition window
  (6 months per REGISTRY_RECOVERY.md Scenario B).
- After window expires, old key is distrusted; users who
  haven't updated get a *forced* warning to self-update.

**Done when:** a planned key rotation can be executed without
breaking existing installs; the transition window is
documented; the rotation runbook from
REGISTRY_RECOVERY.md extends to cover binary key sync.

## Open questions

1. **OS package maintainership.**  Homebrew tap + apt repo
   require ongoing volunteer effort.  Maintain in-house from
   the start, or wait for community demand?  Recommendation:
   in-house from the start for Homebrew (low overhead, broad
   reach on macOS); apt/dnf later.

2. **Pre-release channels.**  Should there be a `nightly` and/or
   `beta` channel alongside `stable`?  Mirrors Rust.  Useful for
   testing pre-release fixes against advisories without breaking
   stable users.  Recommendation: skip for 1.0 — single
   `stable` channel; add `beta` only when release cadence warrants.

3. **Installer hosting.**  loft-lang.org domain registration +
   HTTPS cert + static page hosting needs setup.  Recommendation:
   GitHub Pages from a small static repo (`loft-lang/loft-lang.org`)
   is the cheapest, audit-friendly path — both the page AND the
   install.sh script live in a public Git history.

4. **`curl | sh` ergonomics vs paranoia.**  Some users won't
   `curl | sh`.  Offer also `curl ... -O install.sh && less install.sh
   && bash install.sh` as the alternative.  Recommendation:
   document both in the README; the script is small enough to be
   auditable by the user before running.

5. **Embedded key vs fetched key.**  Should the running loft
   binary trust ONLY the key it was built with, or also fetch
   a current key from a known URL?  Recommendation: ONLY the
   embedded key.  Fetching a key destroys the whole trust chain
   (the key URL becomes the attack target).  Rotation is via
   Phase 30.5's transition window — new releases carry the new
   key, old releases trust the old key, both work during the
   overlap.

## Dependencies

- **@PLAN12 § Phase 6.7** (security advisory channel) — produces
  the yank signals that `loft self-update` reacts to.  30 +
  6.7 together close the loop; either alone is half a system.
- **@PLAN12 § Phase 6.6** (auto-install on `use`) — pulls in
  the same lockfile + cache primitives `loft self-update` reuses.
- **PKG_REGISTRY.md** — the signed-index infrastructure that 30
  extends with toolchain entries.
- **REGISTRY_BOOTSTRAP.md** — the trust-root operations runbook;
  30.5 extends this to cover binary key sync.
- **REGISTRY_RECOVERY.md** — the incident runbooks; 30.5's
  rotation window is a Scenario B execution.

## Sequencing relative to other open work

- 30.1 (reproducible builds) is the lowest-cost first step.  Can
  ship anytime; no user-visible change until 30.2/30.3 land.
- 30.2 (signed registry entries) blocks 30.3 (installer can't
  trust without entries).
- 30.3 (installer) is the user-visible v1: "`curl | sh` works."
- 30.4 (self-update) is the security loop closer.
- 30.5 (key rotation) is operational hygiene; doesn't gate v1.

Estimated path: 30.1 (M, ~3-5 days) → 30.2 (S, ~1 day) →
30.3 (S-M, ~2-3 days) → 30.4 (S, ~1-2 days) → 30.5 (M, ~2-3
days with documentation).  Total ~10-15 work-days spread
across multiple release windows.
