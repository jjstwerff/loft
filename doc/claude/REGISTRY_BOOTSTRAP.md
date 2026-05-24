<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Registry bootstrap — first-time setup for `loft-lang/registry`

PKG.REG R3 + R3.5.  This is the **one-time** procedure to bring the
file-based registry online.  Run by a loft maintainer when ready to
publish the first package.

Detailed design lives in [PKG_REGISTRY.md](PKG_REGISTRY.md); this
doc is the runbook.

---

## Prerequisites

- GitHub admin access to the `loft-lang` org.
- `ed25519-dalek` 2.x via `cargo install` or a small standalone
  script — same crate the loft binary uses.
- A clean, **offline-capable** machine for the key generation step
  (the private key MUST NOT touch a networked development laptop).

---

## Step 1 — Generate the trust-root keypair (offline)

On an air-gapped or single-purpose machine:

```sh
# Tiny one-off Rust binary; ~10 lines.  Generates the keypair, prints
# the public key as hex, writes the private key to a file.
cargo install --git https://github.com/loft-lang/loft-keygen-sketch \
  loft-keygen
loft-keygen generate \
  --out-private  registry-signing-key.bin \
  --out-public-hex registry-signing-key.pub
```

Outputs:

- `registry-signing-key.bin` — 32-byte raw private key.  **Never
  uploaded to any service.**  Stored:
  - On a hardware token (YubiKey OpenPGP or similar) — preferred.
  - In an offline password manager backup.
  - One additional offline copy on a fresh USB stick locked in a
    safe.
- `registry-signing-key.pub` — 32-byte public key, hex-encoded (64
  characters).  Public; goes into source.

---

## Step 2 — Embed the public key in the loft binary

Edit `src/registry_keys.rs::TRUSTED_PUBLIC_KEYS` (this repo).  Add
the new entry as a 32-byte literal:

```rust
pub const TRUSTED_PUBLIC_KEYS: &[[u8; 32]] = &[
    [
        0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef,
        // ...remaining 24 bytes from registry-signing-key.pub...
    ],
];
```

(The `loft-keygen` helper can emit the literal pre-formatted: `loft-keygen format --in registry-signing-key.pub`.)

Open a PR in the loft repo, get review, merge.  The next loft
minor release ships with the embedded key.

---

## Step 3 — Bootstrap the `loft-lang/registry` repo

1. Create `loft-lang/registry` on GitHub (public, MIT or CC0 — it's
   metadata).
2. Initial layout (mirrors PKG_REGISTRY.md § Anatomy of the
   `loft-lang/registry` repo):

   ```text
   loft-lang/registry/
   ├── README.md
   ├── index.json
   ├── index.json.sig            (filled in by the CI workflow)
   ├── schema/
   │   └── index-v1.json         (JSON Schema; lints PRs)
   ├── tools/
   │   ├── validate.py           (PR validation)
   │   └── add_version.sh        (helper)
   └── .github/
       └── workflows/
           ├── pr-validate.yml   (lint + sha256 verify + reproducible-build re-check)
           └── sign-and-commit.yml (post-merge: re-sign index.json)
   ```

3. Seed `index.json` from `doc/claude/registry_sample.json` in the
   loft repo (drop the `_comment` field; set `packages` to `{}` if
   no real package is ready yet).
4. Configure branch protection on `main`: required PR review,
   required CI passing, signed commits.
5. Add the private signing key as a GitHub Actions secret:
   `REGISTRY_SIGNING_KEY_BASE64` = base64 of
   `registry-signing-key.bin`.
6. The `sign-and-commit.yml` workflow runs post-merge to `main`:
   reads `index.json`, signs it with the secret, commits
   `index.json.sig` alongside.

---

## Step 4 — First publish

Per PKG_REGISTRY.md § Publishing flow, run by a package author:

1. `cd loft-crypto && git tag v0.1.0 && git push --tags`
2. `loft package`  (produces `crypto-0.1.0.tar.gz` + sha256 + size).
3. `gh release create v0.1.0 crypto-0.1.0.tar.gz`
4. Open PR against `loft-lang/registry`:

   ```diff
    "packages": {
   -  }
   +  "crypto": {
   +    "description": "SHA-256, HMAC, base64.",
   +    "homepage": "https://github.com/loft-lang/loft-crypto",
   +    "categories": ["crypto"],
   +    "yanked": [],
   +    "versions": {
   +      "0.1.0": {
   +        "url": "https://github.com/loft-lang/loft-crypto/releases/download/v0.1.0/crypto-0.1.0.tar.gz",
   +        "sha256": "43ebf109b0206bd00cab03209b1da081ba9f5caa416aa077ccb1dda65da67cfa",
   +        "size": 5717,
   +        "loft": ">=0.8",
   +        "deps": {},
   +        "conflicts": [],
   +        "replaces": [],
   +        "provides": [],
   +        "binaries": {},
   +        "prerelease": false,
   +        "published": "2026-05-24T08:00:00Z"
   +      }
   +    }
   +  }
    }
   ```

5. CI runs `validate.py` against the PR: schema lint, sha256 verify
   (downloads the release tarball, hashes it, compares), and the
   reproducible-build re-check (clones the source, runs
   `loft package`, compares).
6. Maintainer reviews + merges.
7. `sign-and-commit.yml` regenerates `index.json.sig`.

Now `loft install crypto` works for everyone.

---

## Step 5 — First key-rotation drill (R10.5)

Before the key is needed in anger, run a drill while no real
compromise is happening:

1. Generate a new keypair (Step 1).
2. Embed the new public key alongside the old one in
   `TRUSTED_PUBLIC_KEYS`.  Ship a loft release.
3. Add the new private key as a GitHub secret next to the old one.
4. Update `sign-and-commit.yml` to sign with both keys (produce
   `index.json.sig` containing concatenated signatures; client
   verifies any).
5. Wait 3 months for the ecosystem to adopt the loft release with
   both keys.
6. Drop the old key from `sign-and-commit.yml`.  Drop the old key
   from `TRUSTED_PUBLIC_KEYS` in the next loft release.
7. Old private key: securely destroyed (overwrite + physical
   destruction of any backup media).

Document any issues that emerged during the drill in this file under
"Lessons learned" so the real rotation (when needed) is smoother.

---

## Trust-root recovery

If the private key is lost or compromised:

* **Lost (no compromise):** generate a new key.  Ship a loft
  minor release embedding the new key AND keeping the old key
  (to keep existing signed indexes valid).  Re-sign `index.json`
  in the next CI run.  Schedule old-key removal in a subsequent
  release.
* **Compromised:** generate a new key.  Ship a loft minor with
  ONLY the new key embedded — the compromised key is distrusted
  immediately.  Re-sign `index.json` with the new key in CI.
  Existing client installs that haven't upgraded continue to
  function (`loft.lock` pins the tarball sha256 directly, which
  the compromised key can't forge); new installs require the
  binary upgrade.  Communicate via every loft user-facing channel
  + a CVE if appropriate.

---

## What this doc replaces

Before this MVP, the registry was an "open work" item with no
concrete bootstrap path.  This runbook closes the gap: a maintainer
with admin access can stand the registry up in an afternoon, ship a
loft release with the embedded key, and be ready for the first
publish.
