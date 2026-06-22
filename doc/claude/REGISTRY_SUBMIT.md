<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Submitting a library to the loft registry

This is the author-facing guide for publishing a loft library —
adding a new package, releasing a new version of an existing
package, or yanking a broken release.  Companion docs:

- [PKG_REGISTRY.md](PKG_REGISTRY.md) — registry design + schema reference.
- [PACKAGES.md](PACKAGES.md) — `loft.toml` package format.
- `loft-lang/registry/README.md` — the registry repo's own
  landing page (lives in the registry, not here).

If you're a *consumer* (`loft install <name>`), this doc isn't
for you — just run the command.  Read on only if you maintain a
library you want others to install.

---

## Prerequisites

Before you can submit, you need:

1. **A loft package** — a directory containing a valid
   `loft.toml` and source.  See
   [PACKAGES.md § Package layout](PACKAGES.md#package-layout) for
   the minimum.  In brief:

   ```text
   my-lib/
   ├── loft.toml          # [package] name, version, loft, repository
   ├── src/<name>.loft    # entry point (or [library] entry = "...")
   ├── tests/             # optional, but expected
   └── native/            # optional cdylib if you have native code
   ```

2. **A public source repo on GitHub** — at minimum, the
   reproducible-build re-check (gate 3 below) needs to clone the
   tag and run `loft package`.  The repo URL becomes the
   package's `homepage`.

3. **A loft binary** ≥ the version your package requires.
   Build from [github.com/loft-lang/loft](https://github.com/loft-lang/loft)
   if your distro doesn't ship one new enough.

You do NOT need:

- A signed-commit setup (the registry maintainer signs the
  index; you sign nothing).
- A GitHub Action workflow on your own repo (the registry's CI
  does the validation).
- An account on any registry server (the MVP is a static
  GitHub repo; you submit via PR).

---

## The five-step submit flow

### 1. Tag the release in your source repo

The tag scheme depends on the repo layout.  The loft-lang libraries ship as
**domain monorepos** (`loft-libs-core`, `loft-libs-net`, `loft-libs-graphics`,
`loft-libs-game`, `loft-libs-world`, `loft-libs-assets`) — several packages per
repo — so the tag is **`<pkg>-v<version>`** to disambiguate.  A
one-repo-per-package layout uses a bare `v<version>`.

```sh
cd loft-libs-core/                 # the monorepo
git tag crypto-v0.2.1              # MONOREPO:  <pkg>-v<version>
# git tag v0.2.1                   # one-repo-per-package: bare v<version>
git push --tags
```

Tell `loft package` which scheme to emit via `[package] repository` in
`loft.toml`:

```toml
[package]
name = "crypto"
version = "0.2.1"
repository = "loft-libs-core"      # → tag crypto-v0.2.1 at loft-lang/loft-libs-core
                                   #   (a value with "/" is a full owner/repo)
                                   # omit → legacy loft-<pkg> repo + bare v<version>
```

The version in the tag MUST match `[package] version` — the registry's
reproducible-build re-check (gate 3) clones the tag and re-runs `loft package`.

### 2. Build the tarball with `loft package`

```sh
loft package
```

Output in cwd:

```text
Package created:
  tarball:  my-lib-0.1.0.tar.gz
  size:     <N> bytes
  sha256:   <hex>

Index entry to paste into loft-lang/registry/index.json (PKG_REGISTRY.md schema):
  "0.2.1": {
    "url": "https://github.com/loft-lang/loft-libs-core/releases/download/crypto-v0.2.1/crypto-0.2.1.tar.gz",
    "sha256": "<hex>",
    "size": <N>,
    "loft": ">=0.8",
    "published": "<ISO-8601 UTC timestamp>"
  }
```

The `url` is generated from `[package] repository` (the `<pkg>-v<version>` tag at
that repo); with no `repository` it falls back to `loft-<pkg>/…/v<version>/…`.
The tarball is **deterministic** — same source dir → same sha256 across runs.
This is what gate 3 will re-check.

### 3. Upload the tarball as a GitHub release asset

Use the SAME tag you pushed in step 1 (`<pkg>-v<version>` for a monorepo):

```sh
gh release create crypto-v0.2.1 crypto-0.2.1.tar.gz \
    --title "crypto 0.2.1" \
    --notes "Patch release."
```

The asset's URL — printed by `gh release create` and matching
the `url` field in the index entry above — is what `loft
install` will fetch.

**Don't edit the release assets after this point.**  If you
re-upload a tarball, its bytes change (gzip timestamp, etc.)
and the sha256 in your in-flight PR will no longer match —
gate 2 will reject the PR.  If you need to fix the release,
yank the version and ship `v0.1.1`.

### 4. Open a PR against `loft-lang/registry`

Fork [github.com/loft-lang/registry](https://github.com/loft-lang/registry),
then edit `index.json`:

```diff
   "packages": {
+    "my-lib": {
+      "description": "One sentence on what the library does.",
+      "homepage": "https://github.com/<owner>/<repo>",
+      "categories": ["<category>"],
+      "yanked": [],
+      "versions": {
+        "0.1.0": {
+          "url": "https://github.com/<owner>/<repo>/releases/download/v0.1.0/my-lib-0.1.0.tar.gz",
+          "sha256": "<hex from step 2>",
+          "size": <N from step 2>,
+          "loft": ">=0.8",
+          "deps": {},
+          "conflicts": [],
+          "replaces": [],
+          "provides": [],
+          "binaries": {},
+          "prerelease": false,
+          "published": "<ISO-8601 UTC timestamp from step 2>"
+        }
+      }
+    }
   }
```

Most fields are optional — the minimum is `url`, `sha256`,
`size`, `loft`, `published`.  Drop the empty arrays/objects
you don't use.  See
[PKG_REGISTRY.md § Schema](PKG_REGISTRY.md#schema) for the
full reference.

> **Two things a programmatic edit gets wrong** (learned publishing crypto
> 0.3.3): for a **multi-package repo** (e.g. `loft-libs-core`), copy the existing
> entries' **`subpath`** field (`"subpath": "crypto"`) — `loft package` omits it.
> And if you edit `index.json` with a script, keep **ASCII-escaped JSON**
> (Python's default `json.dump(..., ensure_ascii=True)`): the index escapes
> unicode as `\uXXXX`, so `ensure_ascii=False` rewrites *every* description line
> and buries your one-line change in a full-file diff.
>
> **Maintainer step — re-sign before merge.** An author PR edits `index.json` but
> cannot sign it; before merging, the maintainer re-signs with the trust-root key
> and commits the `.sig` **together with** the index (one atomic change).
> Skipping this leaves a stale signature that fails verification for *all*
> installs (see [REGISTRY_RECOVERY.md](REGISTRY_RECOVERY.md)):
>
> ```
> loft-keygen sign   --in index.json --key ~/.loft/trust-root/registry-signing-key.bin --out index.json.sig
> loft-keygen verify --in index.json --sig index.json.sig --pub "$(cat ~/.loft/trust-root/registry-signing-key.pub)"
> ```

Open the PR.  Title format: `add my-lib 0.1.0` (or for
subsequent versions, `add my-lib 0.2.0`).

### 5. Wait for CI + maintainer review

The registry's CI runs `tools/validate.py` automatically.
Four gates:

| Gate | What it checks | Common failure cause |
|---|---|---|
| Schema lint | Required fields, correct types, `schema_version` unchanged | Typo in field name, wrong type (`size` as string instead of int), forgot `published` |
| Tarball verify | Download `url`, hash it, compare to PR's `sha256` | Re-uploaded the GitHub release asset after opening the PR; pasted wrong sha256 |
| Reproducible-build re-check | Clone `<homepage>` at `v<version>`, run `loft package`, compare sha256 | Source repo's tag points at different bytes than the uploaded tarball; build environment leaked content (e.g. uncommitted files) into the tarball |
| Trigger uniqueness | Every Tier-1 `method:receiver` trigger is owned by at most one package across the whole registry | Your `[triggers]`-enabled package declares a `pub fn` method-on-type (`matches:text`, …) that another package already claims |

If a gate fails, CI surfaces the error as a PR comment.  Fix
the underlying cause, push to your PR branch, CI re-runs.

> **Trigger uniqueness, in plain terms.** If your package opts into
> `[triggers]` (so consumers can call `obj.method()` and have your
> library auto-loaded), every `method:receiver` pair you expose must
> be globally unique — because a consumer writing `line.matches(p)`
> auto-loads *the* package that owns `matches:text`, and there can be
> only one.  `loft publish` warns you locally when a trigger you are
> about to claim is already taken (checked against your cached
> catalog); gate 4 enforces it as a hard reject.  The fix is to rename
> the method, or drop the `[triggers]` opt-in and let consumers reach
> your library with an explicit `use`.

When all four gates pass, a registry maintainer reviews the
PR — typically a sanity check on the description, homepage URL,
and tarball provenance.  After approval, the maintainer signs
the new `index.json` locally (see
[PKG_REGISTRY.md § Why laptop signing](PKG_REGISTRY.md#why-laptop-signing-not-ci))
and merges.

**Once merged, `loft install my-lib` works for everyone.**
Typical time-to-publish from PR open to merge: hours to days
depending on maintainer availability.

---

## Subsequent releases

For a new version after one already shipped (monorepo example, `crypto 0.2.1`):

1. Bump `version` in `loft.toml`.
2. `git tag crypto-v0.2.1 && git push --tags`  (bare `v0.2.1` for a per-package repo).
3. `loft package`.
4. `gh release create crypto-v0.2.1 crypto-0.2.1.tar.gz`.
5. PR adding ONLY the new version row (don't touch the
   existing rows):

   ```diff
       "versions": {
   +    "0.2.0": {
   +      "url": "https://github.com/.../v0.2.0/my-lib-0.2.0.tar.gz",
   +      "sha256": "<new hex>",
   +      "size": <new N>,
   +      "loft": ">=0.8",
   +      "published": "<new timestamp>"
   +    },
        "0.1.0": {
          ...
        }
       }
   ```

The registry **never deletes** version entries.  Old versions
stay so existing `loft.lock` pins keep resolving.  If a
version turns out to be broken or vulnerable, yank it
(below) rather than removing the row.

> **In-tree-tested libraries** (the loft dogfood libs — `graphics`,
> `shapes`, `gridmesh`, `imaging`, `arguments`, `game_protocol`, `web`,
> `hex_world`, `time`) have one extra step: re-sync the loft monorepo
> test fixture so the compiler suite tracks the new tag.  See
> [LIBRARY_AUTHORING.md § 5d](LIBRARY_AUTHORING.md#5d-re-sync-the-loft-monorepo-fixture-in-tree-tested-libs-only).
> A pure registry-only library has no fixture and skips it.

---

## Yanking a broken release

A yanked version stays listed in the index (lockfile pins
still resolve) but new `loft install` calls skip it unless
the user passes `--allow-yanked`.

To yank `v0.1.2`:

```diff
   "my-lib": {
     ...
-    "yanked": [],
+    "yanked": ["0.1.2"],
     "versions": {
       ...
```

PR title: `yank my-lib 0.1.2 — <reason>`.  Use the PR body to
explain why (security issue, broken build, packaging error).
The maintainer will merge after a brief review.

---

## What NOT to include in your package

`loft package` excludes a sensible default list (`.git`,
`target`, `.loft`, `node_modules`, `.vscode`, `.idea`, any
`*.tar.gz` / `*.tar`).  But the responsibility for "what's in
the tarball" is yours.  Common mistakes:

- **Build artefacts in non-standard locations** — anything
  under `target/` is excluded, but if your build leaves
  artefacts under e.g. `out/` they ship in the tarball.
  Inspect with `tar tzf my-lib-0.1.0.tar.gz | sort` before
  uploading.
- **Local config files** — `.envrc`, `.tool-versions`,
  IDE-specific configs.  These don't usually break anything
  but bloat the tarball and confuse downstream consumers.
- **Secrets** — `.env`, credentials.  The registry doesn't
  scan for these; you do.  Use a `.loftignore` if you need
  finer-grained exclusion (planned for a future MVP
  iteration; currently `loft package` only uses the built-in
  list).
- **Test fixtures with private data** — if `tests/` has
  recorded API responses from a service you have credentials
  for, scrub them before tagging the release.

---

## Etiquette

- **Semantic versioning.**  `MAJOR.MINOR.PATCH`.  Breaking
  changes bump major; new features bump minor; bugfixes bump
  patch.  Pre-1.0 minor counts as major for the purpose of
  breakage (you can break in `0.2.0` → `0.3.0`).
- **`loft = ">=X.Y"`** in your version entry should match the
  oldest loft you actually tested against.  Don't claim
  `>=0.8` if you used a 0.8.4-only feature.
- **Deprecation** has no first-class registry support yet.
  Convention: mark deprecated versions yanked with a reason
  pointing at the new package or branch.
- **Multiple maintainers**: file an issue against
  `loft-lang/registry` requesting co-maintainer status.  The
  registry maintainers will add a co-author to the package's
  GitHub repo and update the metadata.

---

## Troubleshooting

### "Tarball sha256 mismatch"

You re-uploaded the GitHub release asset after opening the
PR (or after running `loft package`).  Options:

- Easiest: yank-and-bump.  Delete the v0.1.0 release, bump to
  v0.1.1, re-run from step 1.
- Or: re-run `loft package` against the unchanged source,
  upload the FRESH tarball to the same release, update the
  PR's `sha256` field.

### "Reproducible-build sha256 mismatch"

CI cloned `<homepage>` at `v<version>` and ran `loft
package`, but the resulting sha256 doesn't match.  Causes:

- The tag was force-pushed AFTER you generated the original
  tarball.  Don't force-push tags.
- Your local `loft package` saw files the clean clone doesn't.
  `loft package` now skips files git IGNORES (so leftover
  `tests/_tmp_*.bin` from a `loft test` run no longer leak into
  the tarball), but UNTRACKED-and-not-ignored files and
  uncommitted edits to tracked files still change the bytes.
  Inspect with:

  ```sh
  git status --porcelain   # uncommitted edits + untracked files
  git clean -ndx           # what a clean would remove (untracked + ignored)
  ```

  Commit (or `.gitignore`) anything that shows up, then re-tag.
  `./release.sh` (scaffolded by `loft new`) commits + checks a
  clean tree before tagging, which avoids this.
- Different `loft` versions produce different tarballs (this
  is a known limitation of the MVP — `loft package` should
  pin its output format across versions, tracked as a
  follow-up).  Use the same loft version you'll declare in
  `loft = ">=X.Y"`.

### "Validation says my dep is missing"

Your `[dependencies]` entry references a package not in the
registry yet.  Either submit that dep first, or use a `path =`
local reference (path-deps aren't installable via the registry
but they ARE accepted in the lockfile — the consumer must
fetch them out of band).

---

## Mirroring the registry

The registry is a single static `index.json` on GitHub.  Anyone
can mirror it:

```sh
git clone https://github.com/loft-lang/registry
# host the resulting dir however you like
export LOFT_REGISTRY_URL=https://<your-mirror>/index.json
loft install my-lib
```

A mirror with an unmodified `index.json.sig` works
transparently — clients verify the upstream signature.  A
mirror that wants to use a different signing key needs its
public key added to the loft binary's `TRUSTED_PUBLIC_KEYS`;
file an issue against `loft-lang/loft` to discuss.
