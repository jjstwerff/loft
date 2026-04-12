# Preventing "Failed to grow table" in the deployed browser pages

## Two separate browser pipelines to protect

The repo ships **two independently-built browser artefacts** that land
on GitHub Pages, and each can go stale on its own:

| Artefact | Who uses it | Built by | Make target |
|---|---|---|---|
| `doc/pkg/loft_bg.wasm` + `doc/pkg/loft.js` | `gallery.html`, `playground.html` | `wasm-pack build --target web` | `make gallery` |
| `doc/brick-buster.html` | The featured "click-to-play" arcade game | `loft --html` against a `wasm32-unknown-unknown` libloft.rlib + wasm-opt | `make game` |

Both pipelines produce a wasm/js pair that must agree internally.  The
failure mode is identical: the browser aborts with

```
LinkError: WebAssembly.instantiate(): Failed to grow table
```

or

```
Function import mismatch at 'loft_io' imports
```

because the JS glue and the wasm binary came from different builds of
the source tree.  **Every single time we have seen it** the cause was
a partial or stale rebuild landing in either `doc/pkg/` or
`doc/brick-buster.html` — either on disk during local dev or in git
when a PR skipped the rebuild step.

## Defence layers

Three layers, each individually sufficient; running all three makes
it nearly impossible for a broken gallery to reach users.

### 1. `make gallery` — local one-shot verify-and-rebuild

```
make gallery
```

Cleans `doc/pkg/`, rebuilds via `wasm-pack`, verifies the six assets
the gallery imports, checks timestamps on `loft.js` and
`loft_bg.wasm` agree (the canonical staleness signal), starts a
transient http.server, HEAD-probes every URL the gallery loads, and
only prints `gallery ready` if every step passes.

Fails with `[N/7] ... FAIL ...` pinpointing the stage so the
developer can fix it before pushing.

### 2. PR CI gate (`.github/workflows/ci.yml::gallery` job)

Every pull request runs **both** `make gallery` and `make game` on a
clean ubuntu runner — the two pipelines share the wasm target cache
so the incremental cost is small.  A PR cannot merge if either build
fails, if any asset served by a local http.server returns non-200,
or if the Brick Buster HTML sanity-check fails.

This catches the case where a developer changes loft source, the
graphics library, or `lib/graphics/examples/25-brick-buster.loft`
without rebuilding the corresponding browser artefact.

### 3. Release-time rebuild (`.github/workflows/release.yml::docs` job)

The Pages-deploy job now runs `make gallery` and `make game` in
sequence before publishing.  Pages therefore always serves a wasm
bundle + JS glue generated from the same commit — regardless of what
was committed in `doc/pkg/` or `doc/brick-buster.html`.  This is the
last line of defence if stale files slipped past PR review somehow.

### 4. Runtime guard (`doc/gallery.html::initLoft`)

If a mismatch ever reaches a browser despite the above, the gallery
now translates the cryptic `LinkError: Failed to grow table` into:

> The gallery's WASM bundle and JS glue are out of sync (classic
> "failed to grow table" error). This usually means the deployed
> build is stale.  On a local clone, run `make gallery` to rebuild
> both together.  If you are on the deployed site and still see
> this, please file an issue.

The user sees an actionable message, not a browser internal.

## Why not just `.gitignore doc/pkg/`?

An obvious further step is to stop committing the generated bundle
entirely and only build it at deploy time.  That would also work —
and is the cleanest long-term answer — but it breaks two current
workflows:

- `make serve` immediately after `git clone` with no other setup.
- Forking the repo and browsing `doc/` locally via `file://` URLs
  without running any build.

Removing `doc/pkg/` from git would require everyone to run
`make gallery` before `make serve`, and fork users would see broken
404s.  If the CI layers above prove insufficient in practice, moving
to an ignored-but-rebuilt-on-deploy model is the cleanest next step.

## Recap — what happens on each event

| Event | What catches a broken browser artefact |
|---|---|
| Dev edits, runs locally | `make gallery` + `make game` on demand |
| Dev opens a PR | CI `gallery` job runs **both** `make gallery` and `make game` |
| PR merged to main | CI `gallery` re-runs post-merge |
| Tag pushed, Pages deploys | Release workflow runs `make gallery` + `make game` before `gh-pages` deploy |
| User opens the deployed page | Runtime `explainLoadError` surfaces the classic LinkError as an actionable message in `gallery.html` |

## See also

- `Makefile` — `gallery` target (7-step recovery pipeline)
- `doc/claude/GAME_TESTING.md` — the same approach used for per-example snapshot tests
- `doc/claude/BRITTLE.md` — section 3 (thread/build-glue plumbing) has a similar pattern: raw pointers threaded through multiple sites, easy to desync.
