<!--
This file is shipped to `loft-lang/registry/README.md`.
Copy it as part of REGISTRY_BOOTSTRAP.md Step 3.

The doc/claude/registry_ci_template/README.md is the deploy-side
instruction set (read by the loft maintainer); THIS file is what
ecosystem contributors see when they land on the registry repo.
-->

# loft-lang / registry

The package registry for the [loft](https://github.com/jjstwerff/loft)
language ecosystem.  A single static `index.json` file that the
loft client consumes when you run `loft install <pkg>`.

If you want to install a package, you don't need this repo — just
run `loft install <name>` (or `loft search <query>`) from your
loft project.  The client fetches the index automatically.

If you want to **publish** a package or understand how the
registry works, read on.

---

## How packages get added

Per the design in
[loft's PKG_REGISTRY.md](https://github.com/jjstwerff/loft/blob/main/doc/claude/PKG_REGISTRY.md):

1. **Author tags a release** in their package repo (e.g.,
   `git tag v0.1.0 && git push --tags`).
2. **Author runs `loft package`** in the package directory to
   produce `<pkg>-<version>.tar.gz` plus the sha256.
3. **Author uploads the tarball** as an asset on a GitHub release
   for the tag.
4. **Author opens a PR here** adding a version row to
   `index.json`:

   ```diff
    "crypto": {
      "versions": {
   +    "0.1.0": {
   +      "url": "https://github.com/loft-lang/loft-crypto/releases/download/v0.1.0/crypto-0.1.0.tar.gz",
   +      "sha256": "abc123…",
   +      "size": 5717,
   +      "loft": ">=0.8",
   +      "deps": {},
   +      "published": "2026-05-24T00:00:00Z"
   +    }
      }
    }
   ```

5. **CI runs `tools/validate.py`** automatically on the PR:
   - **Schema lint** — required fields, correct types.
   - **Tarball verify** — downloads the release tarball, hashes
     it, compares to the PR's claimed sha256.
   - **Reproducible-build re-check** — clones the source repo at
     the tag, runs `loft package` from scratch, compares the
     resulting sha256 to the PR's claim.  Catches force-pushed
     tags, mis-uploaded tarballs, and opportunistic supply-chain
     swaps.
6. **A maintainer reviews the PR.**
7. **The maintainer signs the new `index.json` locally on their
   trusted laptop** with `loft-keygen sign`, commits the
   resulting `index.json.sig` to the PR branch, then merges.

`loft install` clients fetch both `index.json` and
`index.json.sig`, verify the signature against the public key
embedded in the loft binary, then proceed with the install.

**Why local signing?**  The private key never lives in GitHub
Secrets; it stays on hardware the maintainer controls.  Trade-off:
maintainer has to be at a keyboard for each merge.  For an
early-stage ecosystem with weekly publishes this is the right
balance — it removes a third-party trust dependency in exchange
for ~30 seconds of human work per merge.

---

## Schema

See [loft's PKG_REGISTRY.md § Schema](https://github.com/jjstwerff/loft/blob/main/doc/claude/PKG_REGISTRY.md#schema)
for the full field reference.  Minimum required per version row:
`url`, `sha256`, `size`, `loft`, `published`.  Everything else
(`deps`, `conflicts`, `replaces`, `provides`, `binaries`,
`prerelease`, `categories`) is optional.

---

## Trust roots

`index.json` is signed with an Ed25519 key held by the loft
maintainers.  The public half is embedded in every loft binary
release (in `src/registry_keys.rs::TRUSTED_PUBLIC_KEYS`).

If you suspect the signing key is compromised, file an issue
ASAP — see [REGISTRY_BOOTSTRAP.md § Trust-root recovery](https://github.com/jjstwerff/loft/blob/main/doc/claude/REGISTRY_BOOTSTRAP.md#trust-root-recovery)
for the response procedure.

---

## Mirrors

Anyone can host a mirror by forking this repo and pointing users
at the fork:

```sh
export LOFT_REGISTRY_URL=https://raw.githubusercontent.com/<your-fork>/registry/main/index.json
loft install crypto
```

The loft client refuses to use a mirror that doesn't have a valid
signature from a key in `TRUSTED_PUBLIC_KEYS` (unless
`--allow-unsigned` is passed).  If you want your mirror to sign
its own index with a different key, file an issue to discuss
adding your key to the loft binary's trust root list.

---

## Yanking

Open a PR that removes the version from `versions` and adds it
to the package's `yanked` array.  Yanked versions stay listed
(so existing `loft.lock` pins still resolve) but new installs
skip them.  Use the PR description to record the reason.

---

## Reporting issues

- **Bad / malicious package**: file an issue here AND in the
  package's home repo.  Maintainers will yank pending
  investigation.
- **Registry bug**: file an issue here.
- **Loft client bug** (install fails, sig verify fails): file
  an issue in [jjstwerff/loft](https://github.com/jjstwerff/loft/issues).

---

## License

Index data (`index.json`) is offered as-is, no warranty.
Tooling (`tools/*.py`, workflows) is LGPL-3.0-or-later.
