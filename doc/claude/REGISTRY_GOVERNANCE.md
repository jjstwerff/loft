// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Registry Governance

Procedures for adding third-party libraries to the central Loft package registry
and for responding when problems are discovered in listed packages.

The registry is a plain text file (`registry.txt`) maintained in the
`jjstwerff/loft-registry` GitHub repository.  The file format is described in
[REGISTRY.md](REGISTRY.md).  This document governs who may add entries and what
happens when an entry must be restricted or removed.

---

## Contents
- [Principles](#principles)
- [Registry Format — Extended Fields](#registry-format--extended-fields)
- [Submission Requirements](#submission-requirements)
- [Review Checklist](#review-checklist)
- [Approval Workflow](#approval-workflow)
- [Native Package Track](#native-package-track)
- [Problem Reporting](#problem-reporting)
- [Severity Classification](#severity-classification)
- [Response Procedures](#response-procedures)
- [Yanking and Deprecation](#yanking-and-deprecation)
- [Author Appeals](#author-appeals)
- [Registry Maintainer Responsibilities](#registry-maintainer-responsibilities)

---

## Principles

1. **Source-visible** — every registered package must have a publicly readable
   source repository.  Binary-only packages are not accepted.
2. **Fast to restrict** — yanking a package is a one-line edit to `registry.txt`
   and takes effect immediately for new installs.  Security response must not
   be slowed by process.
3. **Proportionate** — minor bugs do not trigger yanks.  The response matches
   the severity.
4. **Stable URLs** — a registered URL for a specific version must never change.
   If the file moves, a new version entry is added.  Old entries are not edited.
5. **One maintainer** — at this stage of the project one person (the repository
   owner) holds final approval authority.  The process is designed to be
   lightweight for a small community.

---

## Registry Format — Extended Fields

The base format (`name version url`) is extended with an optional fourth field
to record governance status:

```
# name  version  url  [status[:detail]]
graphics  0.2.0  https://example.com/graphics-0.2.0.zip
graphics  0.1.0  https://example.com/graphics-0.1.0.zip  yanked:CVE-2026-001
opengl    0.1.0  https://example.com/opengl-0.1.0.zip    deprecated:use-graphics
math      1.0.0  https://example.com/math-1.0.0.zip      yanked:malicious
```

### Status values

| Status | Meaning |
|--------|---------|
| *(absent)* | Active — installable without warning |
| `deprecated:<reason>` | Installable but warns; excluded from "latest" selection |
| `yanked:<reason>` | Not installable; excluded from "latest"; existing installs unaffected |

The `reason` field is a short slug used in diagnostics.  It may reference a
CVE identifier, a GitHub issue number, or a brief human-readable label.

### Installer behaviour

| User action | Active | Deprecated | Yanked |
|-------------|--------|------------|--------|
| `install name` (latest) | installs | skipped — next active version is used | skipped |
| `install name@version` (exact) | installs | installs + warning | fails with reason |
| Existing install | works | works | works (no change to local files) |

When a deprecated version is the only available version:

```
warning: graphics 0.1.0 is deprecated (use-graphics).
  No other version is available.  Installing deprecated version.
```

---

## Submission Requirements

A library is eligible for submission if all of the following are true:

### Required for all packages

- **Public source repository** — hosted on GitHub, GitLab, Codeberg, or similar.
  The URL must be provided in the submission issue.
- **Open-source licence** — any OSI-approved licence is accepted.  The licence
  must appear in the repository root (`LICENSE`, `LICENSE.md`, or `COPYING`).
- **`loft.toml` with `name` and `version`** — both fields must be present and
  match the proposed registry entry.
- **Reproducible tests** — `loft --tests <pkg>/tests/` must pass cleanly on the
  submitter's platform.  Test output must be included in the submission.
- **Stable download URL** — the `.zip` URL must remain permanently accessible.
  GitHub release assets, tagged archives, or static file hosting are all
  acceptable.  Direct repository archive URLs (e.g. `github.com/.../archive/`)
  are *not* acceptable because their content can change silently.
- **No name collision** — the package name must not duplicate an existing
  registry entry (including deprecated entries).  If the intent is to supersede
  a deprecated package, contact the maintainer before submitting.

### Additional requirements for native packages

Native packages ship compiled shared libraries and execute arbitrary code inside
the interpreter process.  They require extra scrutiny:

- **Rust source only** — native extensions must be written in Rust.  Pre-compiled
  blobs with no corresponding source are rejected.
- **No `unsafe` outside the plugin boundary** — `unsafe` is permitted only in
  the `loft_register_v1` entry point and in direct FFI calls to platform APIs.
  All other Rust code must be safe.
- **Dependency audit** — the submission must list all crate dependencies and
  their versions.  Dependencies with known CVEs at submission time are a
  blocking issue.
- **Explicit capability declaration** — the submission must state clearly what
  system resources the native code accesses (network, filesystem, GPU, audio,
  etc.).  This is informational, not restrictive, but must be accurate.

---

## Review Checklist

The maintainer works through this checklist before approving:

### Pure-loft packages

- [ ] Source repository is public and readable
- [ ] Licence file is present and OSI-approved
- [ ] `loft.toml` fields `name` and `version` match the submission
- [ ] Download URL is stable (not a mutable archive URL)
- [ ] `loft --tests` passes (submitter-provided output reviewed)
- [ ] No name collision with existing registry entries
- [ ] Package description in the issue makes the purpose clear
- [ ] Package does not re-implement a core stdlib function
      (acceptable if it extends or specialises it)

### Native packages (all of the above, plus)

- [ ] Rust source is public and the entry point matches `loft_register_v1`
- [ ] `unsafe` is confined to the registration entry point and FFI calls
- [ ] Cargo.toml dependencies list reviewed; no known-vulnerable versions
- [ ] Capability declaration matches what the code actually does
- [ ] At least one reviewer other than the submitter has read the Rust source
      (the maintainer counts; community review is welcome but not required)

---

## Approval Workflow

### Step 1 — Open a submission issue

The package author opens a GitHub issue in `jjstwerff/loft-registry` using the
**Package Submission** template.  Required fields:

- Package name and version
- Download URL (the exact `.zip` URL)
- Source repository URL
- Licence identifier (e.g. `MIT`, `Apache-2.0`, `LGPL-3.0-or-later`)
- Brief description (1–3 sentences)
- Test output paste or link to a CI run
- For native packages: capability declaration and dependency list

### Step 2 — Community review period

The issue remains open for **7 calendar days** before the maintainer makes a
decision.  Community members may:

- Report concerns (security, name confusion, licence issues)
- Confirm they tested the package successfully
- Suggest improvements to the submission

The 7-day period may be waived by the maintainer for:
- A patch to an already-approved package (same name, new version)
- A dependency of an already-approved package

### Step 3 — Maintainer decision

After the review period the maintainer either:

- **Approves** — adds the entry to `registry.txt` via a pull request, closes
  the issue with a link to the commit.
- **Requests changes** — lists specific blockers in the issue.  The author
  addresses them and re-requests review.  A new 7-day period does not restart
  unless the maintainer judges the concerns were substantial.
- **Rejects** — closes the issue with a written reason.  Rejection reasons
  include: name collision, licence incompatibility, fails to build or test,
  native package fails the safety checklist, or the package duplicates
  existing stdlib functionality without adding value.

### Step 4 — Ongoing versions

Once a package is approved, the author may add new versions by opening a
**New Version** issue (lighter template: URL + test output only).  The 7-day
period applies unless waived.  The maintainer verifies the `loft.toml` version
field increments monotonically and the URL is stable, then appends the new line.

---

## Native Package Track

Native packages (those with `#native` annotations and compiled shared libraries)
follow the same workflow but with a **14-day** review period and a mandatory
Rust source review.  The checklist item "at least one reviewer other than the
submitter has read the Rust source" must be satisfied before the maintainer
approves.

If no community reviewer steps forward in 14 days, the maintainer performs the
source review alone.  This is acceptable for small packages.  Large or complex
native packages may be held until a reviewer is found.

---

## Problem Reporting

Anyone — user, security researcher, or package author — may report a problem by
opening a GitHub issue in `jjstwerff/loft-registry` with the **Problem Report**
label.

Required information:

- Package name and affected versions
- Description of the problem
- Reproduction steps or proof of concept (for security issues: report privately
  first — see below)
- Suggested severity (the maintainer makes the final call)

### Security vulnerabilities — private disclosure

For security issues (malicious code, data exfiltration, privilege escalation,
or any issue where publishing reproduction steps could cause immediate harm),
report privately:

- Email: `<maintainer-email>` (from the GitHub profile)
- Or use GitHub's **private security advisory** feature on the registry repository

The maintainer will yank the affected versions within **24 hours** of a credible
private report, before any public disclosure.

---

## Severity Classification

| Severity | Examples | Target response |
|----------|----------|-----------------|
| **P0 — Critical** | Malicious code, data exfiltration, remote code execution, supply-chain attack | Yank within 24 h; no discussion required |
| **P1 — High** | Data loss, crash in common use path, security issue without active exploit | Deprecate within 48 h; yank if no fix in 14 days |
| **P2 — Medium** | Incorrect output, API incompatibility with a published version, failed tests | Notify author; deprecate if no fix in 30 days |
| **P3 — Low** | Documentation error, minor edge-case bug, cosmetic issue | Notify author; no forced action |

Severity is assigned by the maintainer after reviewing the report.  The reporter's
suggested severity is taken as input, not as binding.

---

## Response Procedures

### P0 — Critical

1. Maintainer yanks all affected versions immediately (single-line edit to
   `registry.txt`, merged without review period).
2. Maintainer opens a public issue describing the problem at a high level
   (no exploit details if not yet public).
3. If the author is reachable and acting in good faith, they are given
   opportunity to release a fixed version before the public issue is opened.
   This window is at most **24 hours**.
4. If the package was malicious or the author is unresponsive, the package is
   permanently removed from the registry (all versions yanked with
   `yanked:malicious` or `yanked:removed`).
5. The public issue references the yank commit and summarises the nature of the
   problem.

### P1 — High

1. Maintainer marks affected versions `deprecated:<issue-number>` within 48 h.
2. Maintainer notifies the package author via the GitHub issue and, if possible,
   via the source repository's issue tracker.
3. Author has **14 days** to release a patched version.
4. If a fix is released and passes the review checklist, the patch version is
   added to the registry and the deprecation reason updated to point to it.
5. If no fix appears in 14 days, the affected versions are yanked.

### P2 — Medium

1. A GitHub issue is opened in the registry repository referencing the problem.
2. The package author is tagged and has **30 days** to respond.
3. If a fix is released within 30 days, the new version is added normally and
   the issue is closed.
4. If no response or fix within 30 days, the affected versions are deprecated.
5. If 60 days pass with no fix, the affected versions are yanked.

### P3 — Low

1. The issue is opened and the author is notified.
2. No forced action.  The issue remains open until the author fixes it or
   closes it as "won't fix".
3. The maintainer may add a deprecation comment in the issue if the bug causes
   significant confusion, but registry entries are not changed.

---

## Yanking and Deprecation

### What yanking does

- The status field for the entry in `registry.txt` changes to `yanked:<reason>`.
- `loft install name` (latest) skips yanked entries.
- `loft install name@version` for a yanked version fails with the reason:
  ```
  error: graphics 0.1.0 has been yanked (CVE-2026-001).
    Install a different version or check the project repository for a fix.
  ```
- Existing local installations are **not removed**.  Yanking affects new installs only.
- A yanked entry is never removed from `registry.txt` entirely — the line
  remains so that users who already have that version can understand why it is
  flagged.

### What deprecation does

- The status field changes to `deprecated:<reason>`.
- `loft install name` (latest) skips deprecated entries and selects the next
  active version.  If no active version exists, the deprecated one is installed
  with a warning.
- `loft install name@version` installs the deprecated version with a warning:
  ```
  warning: graphics 0.1.0 is deprecated (outdated).
    Consider upgrading to graphics 0.2.0.
  ```
- Existing installations are unaffected.

### Permanent removal

In cases of confirmed malicious packages, the entry status is set to
`yanked:removed` and a note is added to the registry changelog.  The URL field
is replaced with a placeholder (`-`) so no download is possible even if a user
edits the status field manually.

---

## Author Appeals

If a package author believes a yank or deprecation was applied incorrectly:

1. Open a GitHub issue in `jjstwerff/loft-registry` titled
   `Appeal: <package> <version>`.
2. Explain why the action was incorrect and provide evidence (fixed code,
   misattributed CVE, etc.).
3. The maintainer reviews within **7 days**.
4. If the appeal is upheld, the status is removed or changed and a new version
   is added if appropriate.
5. P0 yanks (malicious code) are not subject to appeal.

---

## User-Side Verification

Users can check their installed packages against the latest registry at any time
using two commands (see [REGISTRY.md § Installed Package Check](REGISTRY.md)):

```sh
loft registry sync     # pull latest registry.txt from GitHub
loft registry check    # compare installed packages against registry
```

`loft registry check` exits with code 1 if any installed package is yanked,
making it usable as a CI gate:

```sh
# In a CI pipeline — fails if any yanked package is installed
loft registry sync && loft registry check
```

Typical output when a yank is relevant to the user:

```
  utils  0.3.0  YANKED  CVE-2026-001 — run: loft install utils
```

The staleness warning (registry older than 7 days) reminds users to sync
regularly without being an error.

### How yanks reach users

1. Maintainer edits `registry.txt` — adds `yanked:<reason>` to the affected line.
2. The change is committed and pushed to `jjstwerff/loft-registry` on GitHub.
3. Any user who runs `loft registry sync` gets the updated file immediately.
4. `loft registry check` then surfaces the yank in the terminal and in CI.

No action is required from package authors or the loft interpreter itself to
propagate the yank — the registry file is the single source of truth.

---

## Registry Maintainer Responsibilities

The registry maintainer commits to:

- Reviewing new submissions within **14 days** of the review period closing.
- Responding to P0 reports within **24 hours** (may be yanking alone, with full
  disclosure to follow).
- Responding to P1 reports with a deprecation decision within **48 hours**.
- Keeping `registry.txt` in a git repository with a public history so all
  additions, yanks, and deprecations are traceable.
- Publishing a `REGISTRY_CHANGELOG.md` that summarises yanks and deprecations
  in human-readable form for users who want to audit their installed packages.
- Not removing entries from `registry.txt` (only adding status fields) so that
  the file remains a permanent, auditable record.

---

## See also

- [REGISTRY.md](REGISTRY.md) — file format, install flow, version resolution, implementation
- [EXTERNAL_LIBS.md](EXTERNAL_LIBS.md) — package format, Phase 1–3 rollout
- [PACKAGES.md](PACKAGES.md) — package layout and native extension design
