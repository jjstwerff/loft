<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Library compatibility contract — declared floors, verified by the versions they name

> **Status: steps 1-5 BUILT (2026-07-28). Step 5 GATES, but only for a package that
> declares a floor — see below. Steps 6-7 remain.** Steps are ordered
> so each one lands on its own, is useful on its own, and cannot break anything before it.
>
> **Step 0 is BLOCKED and cannot be completed as written** — but its *generalisable* half,
> the retention guard, is BUILT and nightly. See its section.

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
  *(Now guarded — step 0. The loss itself is permanent.)*

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

### Step 0 — restore `web 0.2.2` — **BLOCKED: unrecoverable. Replaced by a guard — BUILT**

Investigated 2026-07-28. It cannot be restored, and attempting to would be worse than the
gap.

The history is exact — and reading it with the guard below **corrected** the attribution
recorded on the first pass. `c8893db5` (06-24) added it; `e471bd26` (07-03) yanked it
**correctly**, leaving it listed as the rule requires; the very next index.json commit,
`d8ff94c` (07-03) — *"sign: commit index.json + regenerate index.json.sig"* — dropped the
entry, and that deletion was its **only** index change. Not a later publish, as first written.
So the retention rule was honoured and then lost as collateral of a **signing** commit that
committed a working-tree deletion nobody decided to make.

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

**What replaces this step — BUILT.** `scripts/registry_retention_check.py`, a nightly job in
`registry-validation.yml`. Restoring one lost tarball was never the point; making the loss
impossible to repeat is.

Two halves, because a version stops resolving in two independent ways and neither check sees
the other's failure:

| half | what it reads | the loss it catches |
|---|---|---|
| history | every revision of `index.json`, oldest-first | a version **leaves** the index |
| artifact | a ranged GET per listed URL | a version is listed but no longer **downloads** |

The nightly matrix could never have caught either: it is built *from* the index, so a deleted
version is simply absent from the work list and no red night appears.

**Measured, and both halves proven able to fail:**

| cell | result |
|---|---|
| real registry, 57 revisions | OK — one drop in the entire history, the known one |
| real registry, 99 listed versions | OK — all 99 resolve |
| a constructed drop in a later commit | **FAIL**, naming the revision it was last listed at and the one that dropped it |
| a listed version pointed at a dead asset | **FAIL**, naming the URL and `HTTP 404` |
| unreadable repo | **exit 2**, distinct from a pass — "the guard is broken" must never read as "the registry is fine" |

`web 0.2.2` is the one recorded exemption, printed on **every** run including green ones: an
accepted loss that stops being mentioned is one nobody remembers is still owed. The comment on
`EXEMPT` states the rule that keeps the list from growing — a dropped version is normally
*repairable*, because the tarball URL and its sha256 are still in the history the check just
read, so exemption is only for a loss repair cannot reach.

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

### Step 4 — wire both into library CI as **advice** — **BUILT**

`loft compat check` picks the sample (latest + declared floor + one random interior), reports
its draw, and always exits 0. Wired into `library-ci-reusable.yml` with `continue-on-error`.

**The calibration found a real defect, which is what this step is for.** The first sweep
reported `server` as an API BREAK. It was a **false positive in the differ**: `server` 0.5.0
*added* the method `bound` (plus `listen_on` / `listen_tls`), and `api_diff` treated any
textual change to a struct's member list as a break. Wired to block, a purely additive release
would have failed CI.

Fixed with a finer rule than "additions are free", because that is also wrong: an added
**method** is additive, but an added **field** is not — a consumer writing `Server { … }` must
now supply it. Both directions proven rather than argued: adding a field reports
*"added field `extra_field` (a literal construction must supply it)"*, removing a method that
exists in the baseline reports *"removed method `Server.broadcast`"*, and adding a method
reports drop-in. An unparseable member list still reports "shape changed" — the conservative
direction for a check whose value is that a silent break is impossible.

**Result after the fix: 9 comparable packages, 0 breaks, 17 with only one release installed.**
That quiet baseline is the precondition for step 5.

### Step 5 — flip to blocking — **BUILT**

`loft compat check` exits 1 on a violated floor, and `library-ci-reusable.yml` no longer
carries `continue-on-error`.

**It gates only for a package that DECLARES a floor.** Declaring `api_compatible_with` is the
act of entering the contract: a package that declares nothing has promised nothing, so there
is nothing to enforce. That is the model — a library may break its consumers as long as the
break is an explicit choice — and it is also what makes the flip safe, since no published
package declares a floor today, so gating cannot fail anyone's build until they opt in.

**Raising the floor must not buy silence.** The first implementation dropped sub-floor releases
from the comparison, so bumping the number made every check go quiet — the reflex that turns
floors into decoration. Sub-floor releases are now still compared and reported as a
`DECLARED BREAK`. Keeping the promise is the quiet path; withdrawing one is the loud path.

Four cells, all verified:

| source | floor | result |
|---|---|---|
| break | none | advisory, exit 0, nudged to declare one |
| break | below the break | **FAIL**, exit 1 |
| same break | raised past it | exit 0, prints `DECLARED BREAK` |
| unmodified | declared | exit 0 |

### Step 5a — the three levels, required before a register/PR — **DESIGN**

A library declares **three** compatibility levels, all real versions:

| field | means | status |
|---|---|---|
| `loft` | which loft this needs | exists; all 25 packages declare it |
| `api_compatible_with` | oldest own release this is a drop-in for | built (step 1) |
| `data_compatible_with` | oldest own release whose data this still reads | built (step 1) |

**All three must be present before a package may be registered or open a registry PR.** They
are what a consumer needs to decide whether an upgrade is safe on each of the three axes it
can be hurt on: the platform, the call sites, the stored data.

Not yet enforced. The gate belongs where the registry entry is produced (`loft package`) and at
the registry PR (`registry_validate.sh` / `SUBMITTING.md`), not in the library's own CI —
that CI must keep passing for a package that has not opted in, which is step 5's whole shape.

**The incentive to protect, in both this and step 5:** raising a floor is a promise withdrawn,
and should read like one. The constant updating is the chore, not the default — a library that
raises its floor most releases has taught its consumers that its numbers mean nothing.

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
