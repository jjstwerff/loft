<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# MOVING.md — transferring loft into the `loft-lang` org

A one-time runbook for moving `jjstwerff/loft` (and `jjstwerff/loft-registry`)
into the **`loft-lang`** organisation, so one org owns the whole stack — `loft`,
`registry`, the `loft-libs-*` chunk repos, and `plans`. That unlocks the
**no-copy shared model**: org-default GitHub workflows, one `loft-lang/.github`
repo for issue templates + labels, and a single ownership/permissions model — see
[ISSUE_TRACKING.md § Convention](ISSUE_TRACKING.md) and
[LIBRARY_AUTHORING.md § 5e](LIBRARY_AUTHORING.md).

## Why move

The libraries already live under `loft-lang` (`loft-libs-graphics`, …); only the
compiler repo and the registry sit under a personal account. With everything in
one org:

- **org-default + reusable workflows** (the `apply`/`strip` fixed-pending-merge
  pair, `library-ci`) are defined **once** in `loft-lang/.github`, not copied per
  repo — kills the cross-owner wrinkle and the duplication burden.
- one **`.github` repo** serves issue templates, labels, CONTRIBUTING org-wide.
- uniform permissions (org teams) across loft + libs + consumers.

**Why now, not "ahead of need".** This is not speculative collaboration tooling.
It clears the **structure floor** ([GOALS.md § The two floors](GOALS.md)) that the
dogfood consumers are *already stalled behind* — moros / dryopea / bumper wait on
the cross-project library structure settling. So the move is on the critical path
to *resuming* the user-driven loop, not a bet on contributors who haven't arrived:
the same work that lets multiple actors fix bugs concurrently is the work that
unblocks the games. Keep the cross-repo ceremony light (the script + out-of-tree
isolation do that) so it gets *out of the way* of that loop rather than becoming
the work.

## What transfers for FREE (GitHub native — no action, no breakage)

- **Issues, PRs, stars, watchers, releases, tags, wiki, full history** move with
  the repo. **Issue numbers are preserved**, so every `@GH###` ref (e.g. `@GH252`)
  still resolves at the new location.
- The old **web and git URLs auto-redirect** (`github.com/jjstwerff/loft`, clone,
  fetch, push). The few people tracking it keep working with zero changes — their
  existing clones fetch through the redirect.
- **Do NOT create a stub repo at the old name.** GitHub's transfer redirect is
  automatic and covers `git`, not just the web — *better* than a manual stub. A
  new repo at `jjstwerff/loft` would **shadow and disable** the redirect. Leave
  the old name empty; the redirect lives until/unless someone recreates it (your
  namespace — just don't).

## What does NOT redirect — the real gotchas (check each)

| Gotcha | Why | Action |
|---|---|---|
| **GitHub Pages** | `jjstwerff.github.io/loft` (playground/gallery/brick-buster, 14 refs) is **owner-scoped** and does **not** redirect. | Set a **custom domain** (moves with you) — best — or accept `loft-lang.github.io/loft` and rewrite the 14 refs. The release workflow's `peaceiris/actions-gh-pages` re-publishes under the new owner automatically. |
| **Org Actions permissions** | The `apply`/`strip` workflows need `issues: write` from `GITHUB_TOKEN`; orgs often default workflow permissions to **read-only**. | Enable read/write (or the `issues: write` scope) in `loft-lang` org → Settings → Actions, else the lifecycle automation silently no-ops. |
| **`registry.rs` raw URL** | `raw.githubusercontent.com/jjstwerff/loft-registry/main/registry.txt` — raw URLs are not guaranteed to redirect, and the repo is **renamed** `loft-registry`→`registry`. | Update `src/registry.rs:10` (the rewrite script does this). |
| **Collaborators / secrets / branch protection / environments** | Access becomes org-team-based; some settings re-set on transfer. | Re-grant via org teams; re-check repo secrets, branch protection, and environments after the move. |

## The 155 references — can we automate? Yes, but NOT a blanket replace

A naive `s/jjstwerff/loft-lang/` is **wrong** — the reference scan shows four
distinct cases:

1. **The loft repo** — `jjstwerff/loft` / `github.com/jjstwerff/loft` (~60 refs)
   → `loft-lang/loft`. **Safe to automate**, but the regex must NOT match
   `jjstwerff/loft-<suffix>` (a `-` after `loft` means a *different* repo).
2. **The registry — RENAMED** — `jjstwerff/loft-registry` → `loft-lang/registry`
   (note the name change). Explicit mapping, done first.
3. **Pages** — `jjstwerff.github.io/loft` → custom domain **or**
   `loft-lang.github.io/loft` (a decision; see gotchas).
4. **Repos that are NOT (necessarily) moving** — `jjstwerff/dryopea`,
   `jjstwerff/Dryopea`, `jjstwerff/eagleviewer` (consumers) and the
   **library-package repos** (`jjstwerff/loft-graphics`, `-shapes`, `-server`,
   `-web`, `-game-protocol`, `-game-client`). These must **not** be blindly
   rewritten — the library refs also expose a **pre-existing naming
   inconsistency** (see below) that needs a canonical decision first.

So the automation is a **curated, ordered mapping** (`scripts/rewrite-org.sh`),
specific→general, with the non-moving repos left alone and **reported** for
manual review. Running it both rewrites the safe cases and surfaces exactly the
references that need a human decision.

```
scripts/rewrite-org.sh --check    # preview every rewrite + list refs needing a decision
scripts/rewrite-org.sh            # apply the safe mappings, then report the rest
```

## The library-naming inconsistency to settle (do this with the move)

The docs reference library repos three different ways:

- `sync-fixtures.sh` clones from **`loft-lang/loft-libs-<chunk>`** (chunk repos:
  `loft-libs-graphics` holds graphics/shapes/imaging/gridmesh) — the **canonical**
  source.
- `package.rs` release URLs use **`loft-lang/loft-<pkg>`** (per-package).
- some docs say **`jjstwerff/loft-<pkg>`** (stale).

Pick ONE canonical form (recommended: the chunk-repo form `loft-lang/loft-libs-<chunk>`,
matching `sync-fixtures.sh`) and rewrite the others to it. The script lists every
`jjstwerff/loft-<pkg>` hit so you can map each to its chunk. This is a cleanup the
move makes natural — don't let the drift survive.

## References in the libraries and consumers (the cross-repo sweep)

The move's reference surface is **not only in loft**. The chunk repos
(`loft-libs-*`) and consumers (`dryopea`, `bumper`) reference loft in:

- their **CI** that checks out loft source — the snippet originates in loft at
  `src/main.rs` (the generated `repository: jjstwerff/loft` checkout block) and
  in `lib_plans/12-library-extraction/library-ci.yml.example`. **Fix those two
  sources** so newly-generated configs are correct, then sweep existing CI in
  each repo with the same `rewrite-org.sh`.
- doc links and `@GH###` central-tracker references.

`rewrite-org.sh` is portable — run it inside each chunk repo / consumer checkout
to sweep its refs. GitHub's redirect covers them during the transition, so this
is "at leisure", not "or it breaks".

## Order of operations

1. **Transfer** `jjstwerff/loft` → `loft-lang/loft` in the GitHub UI (needs admin
   on the source + create-repo rights in `loft-lang`). Same for
   `jjstwerff/loft-registry` → `loft-lang/registry` (rename in the same step).
2. **Org Actions permissions** — enable `issues: write` (gotchas table) so the
   label lifecycle keeps working.
3. **Pages** — set the custom domain (or accept the new URL).
4. **Collaborators / secrets / branch protection** — re-establish via org teams.
5. **Reference pass in loft** — `scripts/rewrite-org.sh`; settle the
   library-naming map; verify load-bearing code (`registry.rs`, `main.rs`,
   `scripts/idx`).
6. **Cross-repo sweep** — run `rewrite-org.sh` in each chunk repo + consumer;
   land the fixes in their own commits.
7. **Move the workflow defaults** — lift `apply`/`strip`/`library-ci` into
   `loft-lang/.github` as org-default / reusable workflows; thin each repo to a
   stub or rely on org defaults.

## Verification checklist

- `grep -rn jjstwerff` is clean **except** the intentionally-kept non-moving
  repos (`dryopea`/`eagleviewer`) and historical changelog entries you choose to
  keep.
- `make ci` green on `loft-lang/loft`.
- `@GH252` resolves to `github.com/loft-lang/loft/issues/252`.
- `loft install <name>` fetches from the new registry URL.
- the Pages site is reachable (custom domain or new URL).
- a test push to a branch fires the `apply` workflow (org Actions perms correct).
