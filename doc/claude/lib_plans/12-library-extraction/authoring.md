<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Plan-12 — author-side workflow

Part of [@PLAN12 library extraction](README.md).  Covers
**Phase 6.7a** (author-side yank workflow — the missing
companion to [Phase 6.7's consumer-side advisory
classifier](security.md)) and **Phase 6.16** (`loft publish`
CLI — the missing companion to the existing manual registry
PR submission flow).

Together: the symmetric counterpart to
[registry-resolution.md](registry-resolution.md) (consumer
side: `loft install`, `loft update`, auto-install on `use`).
This file covers the AUTHOR side: how does a library
maintainer publish a new version, yank a vulnerable version,
file an advisory?

Companion docs:
- [security.md](security.md) — 6.7 schema this writes
  against; 6.7a is what closes the loop.
- [registry-resolution.md](registry-resolution.md) —
  consumer-side CLI; the install/update/audit story is
  symmetric.
- `REGISTRY_SUBMIT.md` (post-6.13 harvest) — author-facing
  documentation of these workflows.

---

## Phase 6.7a — author-side yank workflow (proposed 2026-05-31)

**Trigger.**  Phase 6.7 ships the CONSUMER side of the
security-advisory channel: the loft binary reads
`advisories.json`, classifies installed versions by severity,
fails or warns appropriately.  What's missing: how does an
author actually file an advisory?  Today's options:

- **Edit `loft-lang/registry/index.json` by hand** to add the
  typed `status` field on the affected version.
- **Edit `loft-lang/registry/advisories.json` by hand** to
  add the `advisories[]` entry.
- **Submit PR + cross-reference GHSA** — manual process; CI
  may or may not verify the cross-reference.

Without a CLI helper + a documented workflow, the channel has
no authoring path.  A vulnerable library may not be yanked
because the author can't remember the schema, doesn't know
where to submit the PR, or doesn't realize the channel exists.
**6.7a closes this.**

**Scope.**

```
loft yank <pkg>@<ver> --severity <tier> --advisory <id> --summary "..." \
                     --affected ">=0.1.0, <0.1.2" --fixed-in "0.1.2"
```

Behaviour:

1. Clone `loft-lang/registry` to a temp directory.
2. Read `index.json`; find the `(pkg, ver)` entry; add the
   typed `status` field (severity + advisory ID + summary).
3. Read `advisories.json`; append a new entry with the
   cross-referenced GHSA, severity, affected range, fix-in
   version, published timestamp.
4. Verify the cross-reference: `index.json`'s `status.advisory`
   matches one of the `advisories[].id` values.
5. Commit on a feature branch.
6. Open a PR via `gh pr create` (if `gh` is available) with a
   templated body covering: why yanked, evidence, fixed-in
   version, link to the GHSA, list of `[reproducer/poc]` files
   if the advisory has any.
7. Print the PR URL.  Author follows up by responding to gate
   feedback + maintainer review.

**Severity tiers** mirror [security.md § Phase 6.7](security.md#phase-67--security-advisory-channel-proposed-2026-05-31):
`security_critical`, `security_high`, `security_low` / `bug`,
`deprecated`.

**GHSA cross-reference enforcement** (validator gate 4):

Extend `tools/validate.py` in `loft-lang/registry`:

```python
def gate_advisory_crossref(idx: dict, advisories: dict) -> None:
    """Every status.advisory in index.json must reference an
    advisory_id that exists in advisories.json's advisories[]."""
    referenced_ids = set()
    for pkg in idx["packages"].values():
        for v in pkg["versions"].values():
            status = v.get("status", {})
            if isinstance(status, dict) and "advisory" in status:
                referenced_ids.add(status["advisory"])
    actual_ids = {a["id"] for a in advisories["advisories"]}
    missing = referenced_ids - actual_ids
    if missing:
        fail(f"index.json references advisories not in advisories.json: {missing}")
```

This blocks PRs where the author updates index.json's
`status` field but forgets to add the matching
`advisories.json` row.

**Author documentation — `REGISTRY_SUBMIT.md` extension:**

```markdown
## Yanking a published version

When a CVE is filed against your library:

1. **Publish a fixed version first.**  Don't yank before the
   fix is available; that strands consumers.  Tag + release
   the fixed version via the standard publish flow.

2. **Run `loft yank`:**

   ```
   loft yank web@0.1.1 \
     --severity security_critical \
     --advisory GHSA-xxxx-yyyy-zzzz \
     --summary "TLS bypass in ws_client_connect" \
     --affected ">=0.1.0, <0.1.2" \
     --fixed-in "0.1.2"
   ```

3. **Review the generated PR.**  Verify the advisory text is
   accurate; add references (NVD link, original report) if
   applicable; rebase if the registry has new commits.

4. **Submit + monitor.**  Validator runs four gates:
   schema lint, advisory cross-reference, tarball verify (no
   change — yank doesn't re-upload), and the new gate 4
   (cross-ref between index + advisories).  Maintainer
   review focuses on advisory wording + severity tier
   accuracy.

5. **After merge,** users running affected versions see the
   classifier output on their next `loft test` /
   `loft script.loft` invocation.  Security-critical yanks
   refuse-to-run; lower tiers warn loudly or quietly.

For non-security yanks (bug, deprecated), severity tiers
adjust the user-facing noise level — see
`security.md` § Phase 6.7 for the table.
```

**Implementation outline (S, ~1 work-day):**

1. **`loft yank` CLI** in `src/main.rs`:
   - Parse args (pkg, ver, severity, advisory id, summary,
     affected range, fixed-in).
   - Use `gh repo clone` or `git clone` to `/tmp/loft-yank-<rand>`.
   - Read + parse `index.json` and `advisories.json` using
     `loft::registry_index::parse_index` and the (new) parallel
     advisories parser.
   - Modify in-place; write back; commit.
   - Open PR via `gh pr create` if available.
2. **PR template** at
   `.github/PULL_REQUEST_TEMPLATE/yank.md` in
   `loft-lang/registry`.  Auto-selected when the PR title
   matches `^yank:`.
3. **Cross-reference gate** in `tools/validate.py` (the
   gate-4 above).  Add to CI workflow.
4. **`REGISTRY_SUBMIT.md` § Yanking** documentation —
   author-facing workflow.
5. **Tests:** valid yank → PR opens cleanly; mismatched
   advisory ID → gate 4 fails; bad severity tier → CLI
   rejects.

**Open questions:**

1. **Authentication.**  Does `loft yank` use `gh` for the
   PR creation (inherits GitHub auth), or its own GitHub
   App?  Recommendation: `gh` — already required for chunk
   release workflows; no new auth surface.
2. **Self-yank vs maintainer-yank.**  Should non-authors be
   able to yank?  E.g. a security researcher discovers a CVE
   in someone else's library.  Recommendation: anyone can
   submit the PR; maintainer (registry committer) reviews
   before merge.  The validator gate ensures the PR is
   well-formed regardless of who submits it.
3. **Yank reversal.**  Can a yank be undone?  e.g. the
   advisory was wrong.  Recommendation: same flow but with a
   `--unyank` flag that clears the `status` field; advisory
   entry stays in history for audit but moves to an
   "archived" subsection.
4. **Severity escalation.**  An advisory starts as
   `security_low` and gets upgraded to `security_critical`
   when more facts emerge.  Recommendation: edit the
   advisory entry in `advisories.json` directly via a normal
   registry PR; new severity takes effect on the next
   24h advisory-feed refresh.

---

## Phase 6.16 — `loft publish` command (proposed 2026-05-31)

**Trigger.**  Today's library publishing flow (recap from
[ci-and-warnings.md § Bringing a chunk to all-green CI](ci-and-warnings.md#bringing-a-chunk-to-all-green-ci--checklist)):

```bash
# Author's flow today (manual, ~5 commands)
cd <chunk>/<pkg>
$EDITOR loft.toml                         # bump version
git tag <pkg>-v<new>
git push origin <pkg>-v<new>
loft package                              # builds tarball
gh release create <pkg>-v<new> <pkg>-<new>.tar.gz --notes "..."
# then manually edit loft-lang/registry/index.json,
# commit, push, open PR, wait for validator gates.
```

Cargo's equivalent is **one command**: `cargo publish`.  Loft
authors feel the friction.

**Scope.**  A `loft publish` CLI that automates the steps
between "tag pushed + CI green" and "registry PR open":

```bash
cd <chunk>/<pkg>
loft publish                              # one command
```

What happens:

1. **Read `loft.toml`** — extract name + version.
2. **Verify CI green** at the tag for the current package.
   Use the GitHub API to check the workflow run status for
   `<pkg>-v<version>` tag.  Refuse if not green (and offer
   `--allow-not-green` for testing scenarios).
3. **Compute sha256 + size** of the released tarball.  Use
   the GitHub release URL pattern; download to a temp file;
   hash.
4. **Verify reproducibility locally** (optional, on by
   default): re-run `loft package` and confirm the local
   sha256 matches the GitHub-released sha256.  If not, refuse
   with "the tarball you uploaded isn't reproducible from
   source; investigate."
5. **Clone `loft-lang/registry`** to a temp dir.
6. **Edit `index.json`** to add the new version entry.
   Preserve formatting (use a registry-aware JSON editor
   that respects the existing indentation + key order
   convention).
7. **Commit on a feature branch** with a templated message:
   "Add `<pkg>` `<version>`".
8. **Open PR via `gh pr create`** with a templated body
   referencing the release URL, sha256, and the chunk's CI
   run that proved the green state.
9. **Print PR URL.**  Author waits for the three validator
   gates; merges manually after green (or via `gh pr merge`
   if a `--auto-merge` flag is passed).

**Behaviour matrix:**

| Invocation | Reads | Writes | Network |
|---|---|---|---|
| `loft publish` | `loft.toml`, GitHub CI status, GitHub release | registry PR | several HTTP |
| `loft publish --dry-run` | as above | nothing | catalog refresh + CI check only |
| `loft publish --allow-not-green` | as above | as above | as above |
| `loft publish --auto-merge` | as above | also merges PR after green | as above |
| `loft publish --pre-release` | as above | adds `0.1.0-beta.1` style entry | as above |

**Why this is small.**  Most of the machinery already
exists:

- `loft.toml` parser — `src/manifest.rs`.
- sha256 / size computation — `loft::install::install_one`
  reuses the same machinery.
- GitHub API access — `gh` CLI binary; subprocess.
- `index.json` editing — `loft::registry_index::write_index`
  (after a small write-back helper is added).
- PR creation — `gh pr create`.

6.16 is the glue + a thin templating layer.  ~1 day of
focused work.

**Tests** (in `tests/publish.rs`):

- Fresh package with CI green → PR opens cleanly; index.json
  has the new entry; sha256 + size match.
- CI not green → refuse; suggest `--allow-not-green` if user
  knows what they're doing.
- Tarball reproducibility check fails → refuse; surface the
  mismatch.
- `--dry-run` → no writes, prints what would happen.
- `--pre-release` → adds `0.1.0-beta.1` entry; validator
  gates pass.

**Open questions:**

1. **Author identity / signing.**  Should `loft publish`
   sign the PR body with the author's identity (Ed25519
   key)?  Mirrors RustCrate's commit-signing recommendation.
   Recommendation: defer until the trust-root infrastructure
   (Phase 30.5) covers per-author keys.  Today's `gh pr
   create` inherits GitHub identity.
2. **Cargo-style `[package] publish = false`.**  Cargo's
   flag to prevent accidental publishes.  Recommendation:
   yes — set `publish = false` in `loft.toml` blocks `loft
   publish`; useful for local-only libraries.
3. **`loft publish --token <github-pat>`.**  Bypass `gh`
   auth for CI environments.  Recommendation: yes — CI
   publishing flows want this.
4. **Per-author rate limiting.**  Should the registry CI
   reject more than N publishes per day per author?
   Recommendation: out of scope here; an operational concern
   for the registry repo's CI policy.
5. **Crates.io-style "ownership."**  Today the registry has
   no per-package owner field; any committer can yank or
   publish-update any package.  Recommendation: file as a
   followup once abuse happens; not pre-emptive.

**Stage A simplification.**  Once 6.16 ships, the `Bringing
a chunk to all-green CI` checklist's step 6 collapses from
"bump → tag → package → release → manual registry PR" to:

```bash
$EDITOR <pkg>/loft.toml          # bump version
git tag <pkg>-v<new> && git push origin <pkg>-v<new>
gh workflow run library-ci       # wait for green
cd <pkg> && loft publish
```

Four commands instead of seven.  Matches `cargo publish`'s
two-command flow modulo loft's tag-then-publish convention.
