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

## The steps

1. **Finalize `loft.toml`** — version bump, the target list (only targets that passed the
   parity gate), and any `[wasm.bridge]` block. Don't publish a target you haven't proven.
2. **Build per target** — produce the artifacts for each claimed target (interp source,
   `--native` rlib, `--native-wasm` rlib, `--html` bridge bundle).
3. **Record sha256 + size** — for each artifact, exactly as `loft install` will recompute
   them. A mismatch here is an install-time hard failure, not a warning.
4. **Add the entry to `index.json`** — the new version, its artifacts, sha256s, sizes.
5. **RE-SIGN `index.json`** — regenerate `index.json.sig` over the edited index with
   `loft-keygen sign` (the maintainer key). **This is the step that breaks everything if
   skipped** (see below). Verify locally with `loft-keygen verify` before opening the PR.
6. **Open the registry PR** — and on the PR branch, ensure the re-signed `.sig` is included.

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
