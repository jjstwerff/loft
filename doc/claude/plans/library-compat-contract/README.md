<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Library compatibility contract — declared floors, verified by the versions they name

> **Status: steps 1-3 BUILT (2026-07-28), advisory only — nothing gates.** Steps are ordered
> so each one lands on its own, is useful on its own, and cannot break anything before it.
>
> **Step 0 is BLOCKED and cannot be completed as written** — see its section.

## The problem, measured

A library can silently break its consumers today. Verified on the current tree:

- **No library repo carries an API baseline** (0 of 8), and `library-ci-reusable.yml` has no
  api-surface step. A signature can change and nothing notices. `api-compat.yml` guards
  *loft's own* surface against baselines bundled in `tests/fixtures/api_compat/`, on PRs to
  loft, and is deliberately non-blocking.
- **Nothing runs a published version's tests against new source.** Library CI runs the
  library's *current* tests — which the same commit may edit, so a break is invisible by
  construction. `revalidate-libs` runs published libs against *new loft*: the other axis.
- **Resolution is version-only.** `registry_index.rs` picks the highest satisfying version
  with no compatibility input, so a consumer asking `foo = ">=0.1"` silently receives a
  release that broke its contract.
- **One retention hole.** `PKG_REGISTRY.md` promises yanked versions stay listed so
  `loft.lock`-pinned consumers don't break. `shapes 0.1.0` and `glb 0.1.1` honour it;
  **`web 0.2.2` is absent from its `versions` map**, so a lock pinned there cannot resolve.

## The invariant

> **A library's declared compatibility floor is a promise about versions that still exist —
> so it must be both verifiable by running what it names, and only ever changed on purpose.**

Two halves, and neither works alone. A floor nobody checks is decoration; a floor whose named
artifacts have been deleted cannot be checked at all.

## Why libraries do not need the absolute promise

loft itself owes **absolute** compatibility: at contract 1 no functioning program ever breaks,
and the language pays for change by carrying both behaviours edition-style
([COMPATIBILITY.md](../../COMPATIBILITY.md)). That obligation covers the **language, stdlib and
runtime** — the floor everyone stands on, which nobody can opt out of and no version of which
can be chosen.

**Libraries are different, and this system is why.** A library breaking its consumers is
survivable as long as two things hold: the break was *declared*, and the *previous version is
still distributable*. A consumer that cannot take the break keeps resolving to the last version
that works for it. So the library's old behaviour is carried by **distribution** rather than by
folding it into the code — which is exactly why libraries never needed the folding machinery
the language has, and why a per-library `contract`-style epoch was the wrong shape.

That is `COMPATIBILITY.md`'s "fold ours, host theirs" made concrete: loft folds, the registry
hosts. It also sets the bar for everything below — the design is not trying to prevent library
breaks. It is trying to make them **impossible to make silently**, and impossible to make
without leaving a working version behind.

## The two numbers

Real versions, not abstract epochs — they mean something on sight, they can appear in
`loft.lock`, and they name artifacts that exist:

```toml
[package]
version              = "0.7.0"
api_compatible_with  = "0.3.0"   # I am still a drop-in for anything >= 0.3.0
data_compatible_with = "0.1.0"   # I still read data written since 0.1.0
```

Separate because the failures differ in kind: an API break costs a recompile, a data break
costs someone's stored file. `hex_terrain` is the worked example — it kept its API and changed
what it computed over stored heights. One number cannot say "safe to upgrade, but migrate".

**Breaking is raising the number**, in the same commit as the break. That edit is the explicit
choice; there is no other way to declare one, and in particular the release version is NOT an
indicator — releases must stay clean and mean only "newer".

## Failure paths (enumerated first, per DESIGN_PROTOCOL)

| # | What happens | Required behaviour |
|---|---|---|
| F1 | API breaks, floor not raised | **Fail.** The unclaimed break. |
| F2 | Floor raised, nothing actually broke | Advice, not failure — a conservative author is not a defect. |
| F3 | The named floor version is not in the registry | **Fail loudly.** An unverifiable claim must never read as a verified one. |
| F4 | Old version's tests fail for a *loft* change, not a library change | Re-classify as environmental (the `revalidate-libs` discipline) or the first language change turns every history red and the check gets switched off. |
| F5 | A random sample fails, the job is re-run, a different version is drawn | **Must reproduce.** Print the drawn version; honour a pin. A real break that vanishes on re-run is worse than no check. |
| F6 | A version's tests can no longer run at all (fixtures gone, dep removed) | Explicit per-version exemption, recorded — never a silent skip. |
| F7 | Window is huge and suites are slow | Bounded per run by construction (see cost), with a budget at the release gate. |
| F8 | Data format breaks, `data_compatible_with` not raised | **Fail** — same rule as F1 on the other axis. |

## Cost, and why it is bounded

Measured today: median 2 published versions per package (mean 2.9, max 10), a pure-loft suite
runs in 0.06–0.12 s, and 6 of 25 packages carry a native crate needing a cargo build per
version. **But today's numbers are a snapshot of a young registry** — mature libraries mean
more versions and slower suites, so the design must not depend on them.

So cost is **O(1) per PR** by construction:

| when | what runs | cost |
|---|---|---|
| every PR | new + floor + latest + **one random interior version** | 4 suites, flat |
| release / ship gate | the full window, under a time budget | O(k), rare |

The random interior sample is what makes it mean: a break cannot hide in a version nobody
looks at. Coverage accumulates — with a 10-version window, ~29 runs puts any given version
under 5% chance of never having been sampled.

Cost is proportional to the *claim*: a library declaring a floor 20 releases back is asserting
something large and pays to verify it; one that raised its floor last release pays nothing.
That is the correct incentive, and narrowing the claim is always available.

**On failure, bisect** (O(log k), only on the failing path) and report the oldest version that
still passes — that is what the floor should have said. The check then sets the number for the
author rather than merely accusing them.

## Steps

Each step is independently landable and verifiable. Nothing gates until its noise has been
measured — the lesson of every check in this repo that had to be walked back.

### Step 0 — restore `web 0.2.2` — **BLOCKED: unrecoverable**

Investigated 2026-07-28. It cannot be restored, and attempting to would be worse than the
gap.

The history is exact. `c8893db5` (06-24) added it; `e471bd26` (07-03) yanked it **correctly**,
leaving it listed as the rule requires; `738984bc` (07-07), an unrelated `web 0.3.0` publish,
silently dropped the entry. So the retention rule was honoured and then lost as collateral of
a later publish — nobody decided to remove it.

But the artifact is gone too: the release asset 404s, and **no `web-v0.2.2` tag exists** on
`loft-libs-net`. Source, tag, release and index entry are all erased; only the `yanked` marker
survives, which now *implies* a version that exists and is discouraged when in fact there is
nothing to install.

Recreating it is not an option worth taking: without the tag there is no source to package,
and republishing a rebuild that did not reproduce sha256 `59518a5…` would substitute different
code under a version consumers may trust — a silent substitution is worse than an honest
absence. The one consumer that needed it, `routing`, already survived by **vendoring** the
source into `routing/lib/web/` — which is what people do when distribution cannot be relied on,
and is the clearest evidence available that this gap has a cost.

**What replaces this step:** a retention guard, so it cannot recur — a check that no version
ever leaves `index.json`, and that every listed version's artifact actually resolves. That is
the generalisable fix; restoring one lost tarball was never the point.

### Step 1 — parse the two fields, inert

`api_compatible_with` / `data_compatible_with` in `[package]`, validated as versions, exposed
on the manifest. Nothing reads them. Additive optional fields: a manifest without them parses
exactly as today. **Verify:** both forms parse; a malformed value is rejected loudly (never
silently ignored — the `check_version` lesson, where an unparseable bound was accepted as
"any").

### Step 2 — `loft compat api <published-version>`, advisory — **BUILT**

Fetch that version's source, diff its API surface against the working tree, report
additive-or-not. Reuses `loft api-surface --check`. A standalone command, no CI. **Verify:**
run it across all 25 published packages and read the output; a known break must be reported.

### Step 3 — `loft compat test <published-version>`, advisory — **BUILT**

Runs a published release's tests against the working tree's source, staged together in a temp
directory. Those tests are the only description of the behaviour written *before* this change
existed — and unlike the working tree's tests they cannot have been edited to match the new
behaviour in the same commit, which is why library CI running the CURRENT tests can never
catch a self-inflicted break.

**The control runs first** (F4): the published tests against their own source, establishing the
corpus can pass at all on today's loft. Three outcomes, all reachable —

| control | subject | verdict |
|---|---|---|
| fail | — | `UNVERIFIABLE` — stale corpus, the working tree is not blamed |
| pass | fail | `BREAK` — released behaviour changed |
| pass | pass | `drop-in` |

**Measured, and it decides the design:** of 55 published versions with a test suite, **51 (92%)
still pass their own tests on today's loft**; 4 are stale (`cbor 0.1.1`, `graphics 0.3.0`,
`hex_terrain 0.1.0`, `markdown 0.1.0`). So F4 — "old failures are mostly loft's fault" — is
NOT the common case, which was the condition that would have sunk this whole design.

**The cell that justifies the step:** inverting the result of `arguments::parse` while keeping
its signature reads `API: drop-in` on the api axis and `BREAK` here. An API diff proves the
SHAPE of a surface; only the old tests prove it still does the same thing.

**One bug the matrix caught.** The staged directory must be named after the package: a test
does `use <name>;`, which resolves by DIRECTORY NAME, so `control/` and `subject/` silently
fell back to the installed copy in `~/.loft/registry` — comparing a release against itself and
reporting drop-in whatever the working tree said. It read green on a deliberate break until
the directories were renamed.

### Step 4 — wire both into library CI as **advice**

Non-blocking, on the 4-version sample. This is the calibration step: measure the noise across
every published package before anything can fail. **Verify:** the advice is quiet on libraries
that did not break.

### Step 5 — flip to blocking

Only once step 4 reads clean. This is the `warning` tier by the established rule — ignoring it
produces a wrong result for someone downstream. **Verify:** an undeclared break fails; raising
the floor makes the same commit pass.

### Step 6 — resolution honours the floors

`loft install` / the resolver pick the newest version whose `api_compatible_with` is at or
below what the consumer holds, instead of simply the highest. **Verify:** a consumer pinned
across a declared break resolves to the last compatible release rather than the newest.

### Step 7 — the release gate runs the full window

Under a total budget; exceeding it fails the release with "narrow the claim or split the
suite", never a silently-truncated prefix reported as proven.

## Re-assertion sites — the brittleness count

Four places must agree on "is this a break": the CI check, the release gate, the resolver, and
`loft compat` run by hand. **N = 4, and a miss at any of them is silent** — which is the
alarm DESIGN_PROTOCOL step 2 exists to raise. The cure is that all four call **one**
computation (the `loft compat` core), never their own. If any step is tempted to re-derive it,
that is the signal to stop and route it back through the shared one.

## What would falsify this design

- **F4 turns out to be the common case.** If most old-version test failures are loft-caused
  rather than library-caused, the check is noise wearing a contract's clothes, and step 4's
  measurement is where that shows up — before anything blocks.
- **The random sample proves unreproducible in practice.** If pinning does not actually make a
  red re-runnable, the check will be treated as flaky and ignored; F5 is the reason the pin is
  a requirement and not a convenience.
- **Authors raise the floor reflexively** to silence the gate, making every release a declared
  break and the floors meaningless. Watch the rate of floor raises after step 5; if a library
  raises on most releases, the check has taught the wrong lesson.
