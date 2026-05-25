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
   ├── loft.toml          # [package] name, version, loft
   ├── src/<name>.loft    # entry point (or [library] entry = "...")
   ├── tests/             # optional, but expected
   └── native/            # optional cdylib if you have native code
   ```

2. **A public source repo on GitHub** — at minimum, the
   reproducible-build re-check (gate 3 below) needs to clone the
   tag and run `loft package`.  The repo URL becomes the
   package's `homepage`.

3. **A loft binary** ≥ the version your package requires.
   Build from [github.com/jjstwerff/loft](https://github.com/jjstwerff/loft)
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

```sh
cd my-lib/
git tag v0.1.0      # match [package] version in loft.toml
git push --tags
```

The tag name MUST be `v<version>` — the registry's
reproducible-build re-check expects this convention.

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
  "0.1.0": {
    "url": "https://github.com/<owner>/<repo>/releases/download/v0.1.0/my-lib-0.1.0.tar.gz",
    "sha256": "<hex>",
    "size": <N>,
    "loft": ">=0.8",
    "published": "<ISO-8601 UTC timestamp>"
  }
```

The tarball is **deterministic** — same source dir → same
sha256 across runs.  This is what gate 3 will re-check.

### 3. Upload the tarball as a GitHub release asset

```sh
gh release create v0.1.0 my-lib-0.1.0.tar.gz \
    --title "v0.1.0" \
    --notes "Initial release."
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

Open the PR.  Title format: `add my-lib 0.1.0` (or for
subsequent versions, `add my-lib 0.2.0`).

### 5. Wait for CI + maintainer review

The registry's CI runs `tools/validate.py` automatically.
Three gates:

| Gate | What it checks | Common failure cause |
|---|---|---|
| Schema lint | Required fields, correct types, `schema_version` unchanged | Typo in field name, wrong type (`size` as string instead of int), forgot `published` |
| Tarball verify | Download `url`, hash it, compare to PR's `sha256` | Re-uploaded the GitHub release asset after opening the PR; pasted wrong sha256 |
| Reproducible-build re-check | Clone `<homepage>` at `v<version>`, run `loft package`, compare sha256 | Source repo's tag points at different bytes than the uploaded tarball; build environment leaked content (e.g. uncommitted files) into the tarball |

If a gate fails, CI surfaces the error as a PR comment.  Fix
the underlying cause, push to your PR branch, CI re-runs.

When all three gates pass, a registry maintainer reviews the
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

For `v0.2.0` after `v0.1.0` already shipped:

1. Bump `version` in `loft.toml`.
2. `git tag v0.2.0 && git push --tags`.
3. `loft package`.
4. `gh release create v0.2.0 my-lib-0.2.0.tar.gz`.
5. PR adding ONLY the new version row (don't touch the
   existing `0.1.0` row):

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
- Your local `loft package` saw files the clean clone
  doesn't (uncommitted changes, files outside the tracked
  set, an `.loftignore` that doesn't match the build's
  exclusions).  Inspect with:

  ```sh
  git clean -fdx        # show what's NOT in git
  ```

  Anything `git clean` reports should either be in `.gitignore`
  / excluded by `loft package`, or committed to the repo
  before re-tagging.
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
file an issue against `jjstwerff/loft` to discuss.
