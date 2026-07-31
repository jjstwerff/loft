<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN78 — implementation design: the toolchain is an artifact in the index we already have

**Status — DESIGN, 2026-07-31.**  Supersedes the phase sizing in
[README.md](README.md), which was written 2026-05-31, before the package trust
chain existed.  The threat model and trust-chain analysis there still stand and
are not repeated here.

## What changed under the plan

The plan sized five phases against a world with no signing infrastructure.  That
world is gone: building the *package* registry built almost every mechanism the
*binary* distribution needs.  Re-audited against the tree, not the prose:

| Phase | Plan's assumption | Today |
|---|---|---|
| 30.1 reproducible builds | to be built | **open** — `make-release.sh` emits `SHA256SUMS` + `stdlib.manifest`, which is integrity, not byte-identical rebuild |
| 30.2 signing + registry entries for binaries | to be built | **mechanism exists** — `registry_index::BinaryEntry { url, sha256, loft_ffi_fp }`, keyed per target triple, already downloaded and sha256-verified by `install.rs` for package cdylibs.  What is missing is an *entry*, not a mechanism |
| 30.3 `install.sh` | to be built | **open** — genuinely new (see § the two asymmetries) |
| 30.4 `loft self-update` | to be built | **half exists** — the verify half is `install.rs`'s existing path; the replace half is new |
| 30.5 key rotation + LTS | M, 2-3 days | **built** — `registry_keys.rs` embeds MULTIPLE Ed25519 trust roots and accepts a signature from any; rotation is "ship a release that adds the new key, ship a later one that drops the old" |
| dependency: @PLAN12 §6.7 advisory channel | blocking | **exists** — `registry_advisories.rs` (`Advisory`, `AdvisoryFeed`, `classify`) |

The plan's own words were *"30 + 6.7 together close the loop; either alone is
half a system."*  6.7 landed.  This is the other half, and it is much smaller
than 10–15 work-days because most of it is already running in production for
packages.

## The invariant

> **The loft toolchain is one more signed artifact in the index, verified by the
> same chokepoint every other artifact goes through.**

A case never tested behaves correctly for the same reason the tested ones do:
`install.rs` fetches a signature-verified index, reads a per-target
`BinaryEntry`, downloads it, and refuses it unless the sha256 matches.  A
toolchain binary is that, with a different destination.

## Re-assertion sites — the brittleness, counted before any code

The invariant must be re-stated at every site that admits an artifact.  Today:

| # | Site | Status |
|---|---|---|
| 1 | `install.rs` package tarball | exists, verified |
| 2 | `install.rs` prebuilt cdylib (`BinaryEntry`) | exists, verified |
| 3 | `loft self-update` | **new — must route through the same function, not a copy** |
| 4 | `install.sh` (bootstrap) | **new — CANNOT route through it: there is no loft yet** |

Site 3 is free if self-update calls the existing verify path; writing a second
sha256 check there is the failure mode, and it is silent (a wrong artifact
installs, nothing errors).  **Site 4 is the irreducible one** — a shell script
cannot verify a signed index.  The cure is not to make the shell smarter but to
shrink what it must be trusted for:

> `install.sh` carries **one pinned sha256** of the bootstrap binary and hands
> off immediately.  It never parses the index, never checks a signature, never
> chooses a version.  Its whole trust surface is one constant a reader can eyeball
> — and the first thing the installed binary does is verify itself against the
> signed index (step 2), so the shell's claim is checked by loft within seconds.

That drives `N × silence` down: sites 1–3 share one chokepoint, and site 4's
omission is loud (the pinned hash is the only thing it does).

## Where the invariant does NOT reach — the two asymmetries

Named because "the toolchain is just another package" is the elegant claim, and
elegant claims are where designs break.  Two places it genuinely does not hold:

1. **Bootstrap has no verifier.**  Installing a package happens *inside* loft;
   installing loft cannot.  Handled by the handoff above — not by pretending the
   shell can do what loft does.
2. **Self-replacement mutates the running program.**  No package install does
   this.  A running binary cannot be overwritten in place on Windows, and on Unix
   unlinking it while running is legal but leaves the process on the old inode.
   The mechanism is rename-then-replace, and it is the one genuinely new piece of
   engineering in this plan.

A third asymmetry is a non-issue and was checked rather than assumed:
`compare_semver` parses `major.minor.patch` as integers, so loft's **calendar**
versions (`2026.7.2`) order correctly with no special case.  `BinaryEntry`'s
`loft_ffi_fp` is a cdylib-ABI gate and must be absent for the toolchain entry.

## The steps

Each is independently landable, each is verified by the one before it, and
nothing is trusted before it is checkable.  Steps 1–3 cannot damage an
installation: they add data and read-only commands.

**Step 0 — remove the false claim (docs, no code).**  DONE 2026-07-31.
`LIBRARIES.md` stated that `loft install` / self-update verifies the release
sha256s.  Neither does: `install.rs` has no toolchain path, there is no registry
entry for the binary, and the sha256s are read from the release's `.zip.sha256`
sidecars purely to be displayed.  It now says what is true — verify by hand with
`sha256sum -c`, no automatic path yet — and points at this plan.  The sentence
lives in `scripts/gen-library-catalogue.py`, since `LIBRARIES.md` is generated and
not committed; the comment there says what to reword when steps 1–2 land.

`RELEASE.md` was checked and left alone: its two mentions sit in a release
checklist that the very next paragraph marks `[build]` (open work), so it
describes a target state and says so.  Correcting it would have been the
over-reach — it was already honest.
*Verify:* every surviving `self-update` mention names future work (`RELEASE.md`'s
open-work paragraph, `ROADMAP.md`'s @PLN78 row).

**Step 1 — make the client tolerate the entry BEFORE it exists.**  DONE 2026-07-31.

*This step was rewritten by its own probe.*  It read "publish a toolchain entry
— no client reads it yet, so this cannot break anything."  That was false, and
the cheapest check said so: `parse_index` propagated a package's parse error with
`?`, so ONE unreadable entry rejected the WHOLE index.  A healthy `regex` beside
one under-specified `loft` entry resolved *nothing*.  Publishing the entry first
would not have been inert — it would have taken registry access away from every
deployed loft until they upgraded.

That is a fragility independent of this plan: one index serves every client, so a
publishing mistake in any single package used to be an ecosystem-wide outage.
`parse_index` now **skips** an unreadable package, reports it on stderr, and
records it in `RegistryIndex::skipped` for a caller that must be strict (a publish
check should refuse to sign a malformed index).  Sound because the signature is
verified over the raw document *before* parsing, so skipping is a choice about
already-authenticated data and admits nothing an attacker controls.  Structural
damage — bad JSON, unsupported `schema_version` — stays fatal, since then nothing
in the document can be trusted to mean what it says.

*Verified:* the original probe now yields `PARSED — 1 packages: ["regex"]` with
the skip reported.  Three guards pin it: the malformed entry is skipped and named,
a clean index skips nothing (the control — a parser that skipped *everything*
would also pass the first), and a structurally broken index is still refused.

**Step 1b — publish the toolchain entry (owner action, cross-repo).**
Now inert, because step 1 shipped first.  Add `loft` to `loft-lang/registry`'s
`index.json` with one `BinaryEntry` per target triple, sha256s from the
`.zip.sha256` sidecars `make-release.sh` already emits, then re-sign.  Two things
the probe pinned that this must respect: `Version` requires `url` / `sha256` /
`size` / `loft` / `published`, none of which a per-target toolchain has a natural
single value for — so decide what artifact the version-level fields name (the
release's source archive is the only candidate that keeps a toolchain entry
semantically identical to a package: source, plus prebuilt binaries per target) —
and `loft_ffi_fp` must be ABSENT, since it gates cdylib ABI compatibility and
means nothing for the toolchain.
*Verify:* an OLD loft still resolves packages from the new index (it will skip the
toolchain entry if it cannot read it, which is what step 1 bought).

**Step 2 — `loft verify-self` (read-only).**
Fetch the signed index, find the entry for the running version + host triple,
hash the running executable, compare.  Report agreement / mismatch / no-entry.
Mutates nothing.  This is the step that proves the whole chain end to end —
signature, entry lookup, hash — with zero risk, and it is independently useful:
it answers "is my binary the one that was published?"
*Verify:* passes on a release binary; fails loudly on a locally built one (that
failure is the positive control — a check that cannot fail is not a check).

**Step 3 — `loft self-update --dry-run`.**
Resolve the newest non-yanked version for the host triple, download it to a temp
path, verify sha256, then report what it *would* replace and stop.  Still no
mutation.  Reuses `install.rs`'s verify path — site 3 of the table above, wired
to the chokepoint rather than copied.
*Verify:* dry-run against a stale local version resolves and verifies; against
the newest, reports "already current".

**Step 4 — self-replacement.**
The one new mechanism: download to a temp file beside the target, verify, then
rename the running binary aside and rename the new one into place, restoring on
failure.  Only now, with 1–3 proven, does anything mutate an installation.
*Verify:* on each target OS, update to a version and back; kill the process
mid-write and confirm the installation is still runnable (the rename is the
atomicity, so a crash leaves either the old or the new binary, never a partial).

**Step 5 — advisory integration.**
`self-update` consults the existing advisory feed and refuses to install a yanked
version, and warns when the *running* version is yanked.  This closes the loop
the plan named: 6.7 produces the signal, self-update reacts.
*Verify:* a yanked test version is refused with its advisory text.

**Step 6 — `install.sh` (bootstrap).**
Deliberately last and deliberately dumb: detect OS/arch, fetch the release
tarball, check the one pinned sha256, unpack, then run `loft verify-self` and
print its verdict.  Nothing else.
*Verify:* on a clean container per target, `curl … | sh` installs a loft whose
`verify-self` passes.

**Step 7 — reproducible builds (30.1), off the critical path.**
Nothing above depends on it.  It upgrades the *meaning* of step 1's sha256 from
"this is the artifact the maintainer uploaded" to "this is the artifact the
source produces", which is the stronger claim, but the chain works without it.
Sequenced last on purpose so it cannot block a user-visible installer.

## Failure paths

Enumerated because writing them down is what surfaced the handoff design above.

| Failure | Consequence | Handled by |
|---|---|---|
| Index unreachable / stale | self-update can't resolve | reuse `install.rs`'s existing cached-index fallback |
| Index signature invalid | a forged index picks the binary | existing `verify_or_explain`; never bypassed for the toolchain |
| sha256 mismatch on the downloaded binary | a wrong/tampered binary installs | step 3's verify precedes step 4's replace, always in that order |
| Trust root empty (fresh tree) | everything "verifies" vacuously | existing rule: signature-verified flows require explicit `--allow-unsigned` |
| Replacement interrupted | unusable installation | rename-based swap; a crash leaves old or new, never partial |
| Downgrade attack (index pinned to an old version) | user silently kept on a vulnerable release | step 5: the advisory feed is consulted against the RUNNING version, not only the candidate |
| `install.sh` served tampered | pinned hash is wrong too | irreducible for bootstrap; mitigated by the script being short, public-git-hosted, and readable, and by `verify-self` re-checking against the signed index immediately after |
| Toolchain entry mistaken for a library | `loft install loft` does something odd | decide explicitly at step 1 whether the entry is installable as a package; the safe default is that `self-update` owns it and `install` refuses it |

## What was probed, and what was not

Run against the tree before writing this, because the plan's own sizing was the
thing most likely to be stale:

* **Survived:** `BinaryEntry` + per-triple `binaries` map exists and is already
  sha256-verified in `install.rs` · `registry_keys.rs` embeds multiple trust
  roots, so rotation (30.5) is built · `registry_advisories.rs` exists, so the
  blocking dependency is satisfied · `compare_semver` orders calendar versions
  correctly.
* **Falsified:** the plan's "30.5 key rotation, M, 2-3 days" and its framing of
  30.2 as unbuilt.
* **Not probed — the residue, and the honest tail:** whether the rename-based
  swap is safe on Windows in practice (step 4's own verification is the probe);
  whether the registry CI can publish a toolchain entry without schema changes;
  whether reproducible builds are achievable for this crate graph at all (step 7
  may discover it is not, which is why nothing depends on it).
