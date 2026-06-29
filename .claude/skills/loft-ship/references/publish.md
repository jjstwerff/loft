# The registry publish runbook

Read this before submitting a library to the registry. Full reference:
[REGISTRY_SUBMIT.md](../../../../doc/claude/REGISTRY_SUBMIT.md) (author-facing 5-step flow),
[PKG_REGISTRY.md](../../../../doc/claude/PKG_REGISTRY.md) (the file-based registry design),
[REGISTRY_RECOVERY.md](../../../../doc/claude/REGISTRY_RECOVERY.md) (trust-root incidents).

## How the registry works (the one fact that explains the foot-gun)

The registry (`loft-lang/registry`) is a static **`index.json`** plus a detached signature
**`index.json.sig`** (Ed25519, 64-byte raw), signed by the maintainer trust-root key
(`~/.loft/trust-root/registry-signing-key.bin` / `.pub`). `loft install` downloads `index.json`
(via the raw-GitHub CDN), **verifies the signature**, then fetches the package and checks its
**sha256 + size** against the index entry. So the index and its signature must always move
together — an index the signature doesn't match makes the whole registry unusable.

## The routine (preferred) — `loft package` → tag → release → publish → touch-sign

Don't hand-run the steps below unless you're debugging — the correctness-critical parts (the
index `sha256`/`size` matching the release tarball, the **atomic re-sign**, the fixture pin) are
exactly where a hand publish goes wrong. Use the routines:

- **Cut the GitHub release** (the easy half, author-triggered): bump `version` in `loft.toml`,
  merge to `main`, then push tag `<pkg>-v<version>`. A release-on-tag workflow (or `release.sh`
  locally) runs `loft package` + `gh release create` + uploads the tarball. *The tag is the only
  thing you do by hand — everything correctness-sensitive is downstream.*
- **Publish + sign** (the hard half, runs on the machine that holds the key):
  - own libs, all at once: `scripts/registry_maintain.sh` — publishes every lib that is
    missing/newer than the index, then signs (it signs **through** `registry-sign.sh`).
  - one registry PR: `scripts/registry-sign.sh --pr N` — shows the diff, re-checks every
    tarball `sha256`, then signs.

**The signing gate (why the key never enters CI).** A fully-automatic "push → signed release" is
impossible: the trust-root key must never leave the maintainer's machine (a wrong/stale signature
breaks *every* `loft install` — see the foot-gun below), so signing is **inherently local and
human-gated**. The gate: the **local key does the crypto**, and a **YubiKey TOUCH is the
confirmation** — on touch the YubiKey types its one-time OTP, which the prompt reads as the "yes."
It **waits 10s for the touch, else falls back to a typed `yes`**. Set `LOFT_YUBIKEY_PREFIX` to your
OTP public-id (first ~12 chars) to pin the gate to *your* key. `--yes` skips it (scripted use
only). So the whole release is: **you push a tag, then you touch your key once** — the routine gets
everything between right.

## The steps — what the routine does under the hood (and the manual fallback)

The registry package is a **source tarball** — `loft package` does **not** build per-target
artifacts (no prebuilt cdylib). Consumers build the native/wasm artifacts at install time
against their own loft, so cross-tree `loft-ffi` matching is a *consumer-side* concern, **not a
publish blocker**. (It does mean a `#native` feature only works on `--native` once the
consumer's loft itself carries the needed compiler support.)

1. **Land the code first.** The lib's PR must be merged to its repo's `main` (the package is
   built from `main`), with the new `version` in `loft.toml`.
2. **`loft package`** (in the package dir) → writes `<pkg>-<version>.tar.gz` and **prints the
   index entry** (`url`, `sha256`, `size`, `loft`). It **omits `subpath` and `deps`** — copy
   those from the package's existing entries (e.g. `"subpath": "crypto"`, `"deps": {}`).
3. **GitHub release + asset** (the tag must match the entry's `url`):
   ```
   gh release create <pkg>-v<version> --repo <org>/<repo> --target main \
     <pkg>-<version>.tar.gz --title "<pkg> v<version>" --notes "…"
   ```
4. **Registry PR** — clone `loft-lang/registry`, then:
   - Add the version under `packages.<pkg>.versions.<version>` (the printed entry **plus**
     `subpath` + `deps` + a `published` ISO-8601 UTC timestamp), and bump the top-level
     `updated`.
   - **Edit the JSON with `ensure_ascii=True`** (Python's default): the index escapes unicode as
     `\uXXXX`, so `ensure_ascii=False` rewrites *every* description line and makes the diff
     unreviewable — keep it to your entry + `updated`.
   - **Re-sign, then verify** (the maintainer key; this is the step that breaks all installs if
     skipped — see below). The routine above (`registry-sign.sh`) wraps these two with the
     touch-gated confirmation; the raw commands are:
     ```
     loft-keygen sign   --in index.json --key ~/.loft/trust-root/registry-signing-key.bin --out index.json.sig
     loft-keygen verify --in index.json --sig index.json.sig --pub "$(cat ~/.loft/trust-root/registry-signing-key.pub)"
     ```
   - **Commit `index.json` + `index.json.sig` TOGETHER** (one atomic change), push a branch,
     open the PR against `loft-lang/registry`.

## The re-sign foot-gun (internalize this)

Editing `index.json` **without** regenerating `index.json.sig` leaves a valid-looking index
with a **stale signature**. Every `loft install` then fails signature verification — *all*
packages, not just the new one — and the locked-baseline test (`baselines_are_locked_in`)
reports **"registry index signature INVALID."** This happened in @PLN84 (a `crypto` version was
merged into the index un-re-signed and broke installs until a re-sign PR fixed it). So: **every
change to `index.json` is followed immediately by a re-sign.** Treat them as one atomic edit.

## The CDN-staleness gotcha (don't misread it as a failed publish)

`loft install` reads `index.json` through the raw-GitHub CDN with roughly a **1-hour cache**
(local cache under `~/.loft/registry/`). Right after a merge, `@latest` may still resolve to
the previous version at some edges. To verify a fresh publish, **install the exact version**
(`loft install <lib>@<version>`) rather than `@latest`; if the pinned version installs and
verifies, the publish succeeded — `@latest` will catch up as the CDN propagates. Don't conclude
the publish failed from a stale edge read.

## Verification before you call it shipped

- `loft-keygen verify` passes on the re-signed index (signature matches).
- `loft install <lib>@<version>` succeeds from a clean cache (sha256 + size check pass).
- The parity gate (in SKILL.md) is green on every target the entry claims.
