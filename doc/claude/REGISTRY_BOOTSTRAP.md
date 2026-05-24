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

On an air-gapped or single-purpose machine (Linux or macOS — the
keygen reads `/dev/urandom`):

```sh
# Build the keygen from the loft repo (ships with the binary as of
# the PKG.REG MVP).  The `registry` feature is on by default.
git clone https://github.com/jjstwerff/loft.git
cd loft
cargo build --release --bin loft-keygen
./target/release/loft-keygen generate
```

Outputs in cwd:

- `registry-signing-key.bin` — 32-byte raw private key, chmod 600.
  **Never uploaded to any service.**  Stored:
  - On a hardware token (YubiKey, etc.) — preferred.
  - In an offline password manager backup.
  - One additional offline copy on a fresh USB stick locked in a
    safe.
- `registry-signing-key.pub` — 64-char hex public key.  Public;
  goes into source.

The same invocation prints to stdout (for copy/paste):

- The public key formatted as a Rust `[u8; 32]` literal — paste
  into `src/registry_keys.rs::TRUSTED_PUBLIC_KEYS` (Step 2).
- The private key base64-encoded — paste into
  `loft-lang/registry`'s `REGISTRY_SIGNING_KEY_BASE64` secret
  (Step 3.5).

### Step 1.5 — Store the private key for the long haul

Laptops fail every 2-5 years on average; the private key must
outlive any single machine.  **Three independent copies, two
physical locations, two media types** is the working rule
(3-2-1 backup adapted for a 32-byte secret).

Recommended layout:

| Copy | Where | Purpose |
|---|---|---|
| **1. Daily working copy** | `~/.loft/trust-root/registry-signing-key.bin` on your trusted laptop, chmod 600, inside FileVault/LUKS-encrypted home | The one you'll actually use day-to-day. |
| **2. Hardware token A** | YubiKey 5 PIV slot, stored at your home/office | Primary recovery — if the laptop dies, re-import to a fresh machine in 10 minutes. |
| **3. Hardware token B** | Second YubiKey, stored at a different physical location (parent's house, bank safe-deposit box) | Insurance against fire / theft / flood at copy #2's site. |
| **4. (Optional) Paper / sealed offline copy** | base64 (44 ASCII chars) printed on paper, sealed in tamper-evident envelope, stored in fire-resistant safe at a third location | "What if both YubiKeys die simultaneously" insurance.  Paper outlasts USB sticks (5-10 yr rot). |

**Do NOT put the key in**:

- Cloud storage (Dropbox, iCloud, Google Drive), even encrypted —
  signals to a third party that a high-value secret exists.
- Cloud-synced password managers (1Password / Bitwarden cloud
  vault) — same reason.  Local-only vaults are fine.
- A git repo (even private), an email to yourself, any chat app
  (those backups end up cloud-synced too).

**Do test the backup once a year.**  Pick a YubiKey or the paper
copy at random, restore to a clean test VM, sign a dummy
`index.json`, verify the signature with the public key.  See
[REGISTRY_RECOVERY.md § Annual recovery drill](REGISTRY_RECOVERY.md#annual-recovery-drill)
for the checklist.

If something goes wrong later (laptop dies, key compromised, key
silently corrupted), follow [REGISTRY_RECOVERY.md](REGISTRY_RECOVERY.md)
— it has step-by-step runbooks for every scenario plus the
multi-key rotation mechanism that keeps users from breaking.

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

(The `generate` command already printed the literal in its
stdout; if you only have `registry-signing-key.pub` and need to
re-emit, run `loft-keygen format --in registry-signing-key.pub`.)

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

See [REGISTRY_RECOVERY.md](REGISTRY_RECOVERY.md) for the full
step-by-step runbook covering:

- **Scenario A** — laptop died, backup recoverable.  ~30 min
  drill; no user-visible impact.
- **Scenario B** — laptop died, no recoverable backup (catastrophic
  loss of all working copies).  Forces a multi-key rotation;
  6-month transition window; still zero user-visible impact if
  handled before users hit `loft update`.
- **Scenario C** — key is COMPROMISED (exfiltrated / stolen).
  Same-day distrust; emergency point release with the bad key
  removed; communicate aggressively.

Quick summary so the bootstrap doc reads complete on its own:

* **Lost (no compromise):** generate a new key, embed alongside
  the old one, ship a loft minor release.  Old key continues
  signing indexes during the transition window so existing
  installs keep working.  Drop the old key in a subsequent
  release.
* **Compromised:** generate a new key.  Emergency loft point
  release embedding ONLY the new key — the compromised key is
  distrusted immediately.  Existing `loft.lock` pins keep working
  (the lockfile records each tarball's sha256, which the
  compromised key can't forge); only fresh installs need the
  binary upgrade.  Communicate via every loft user-facing channel
  + a CVE if appropriate.

The architectural reason both scenarios are recoverable: the
client verifies BOTH the index signature AND each tarball's
sha256.  `loft.lock` records the sha256 — once a build is
locked, no signing-key event can corrupt it.  Only NEW installs
and `loft update` consult the index, and those are gated on the
current signature.

---

## What this doc replaces

Before this MVP, the registry was an "open work" item with no
concrete bootstrap path.  This runbook closes the gap: a maintainer
with admin access can stand the registry up in an afternoon, ship a
loft release with the embedded key, and be ready for the first
publish.
