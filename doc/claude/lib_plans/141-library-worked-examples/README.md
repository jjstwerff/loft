<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN141 — Worked examples for the current libraries

> Tracker: [loft-lang/plans#141](https://github.com/loft-lang/plans/issues/141)
> (`subject:libs`, `status:future` → move to `status:active`). Origin: dryopea's
> `docs/EXAMPLES.md` gate (commits `9cf8a01` + `1f4c35f`, 2026-08-17) — this idea
> rolled out across the loft library + feature ecosystem. Built on branch
> `mac-install-dylib-fix` (alongside @PLN142).

## Status

ACTIVE — stdlib + in-tree libraries done; the distributed-library **gate is now
wired into every library's CI** (Phase-last CI ratchet, see below); the **convention
page is done** (LIBRARY_AUTHORING.md § 2a). First distributed library **`arguments`
DONE** — pushed as `loft-libs-core` branch `mac-worked-examples` (rebased onto
`origin/main`, PR not opened). Four `@ARG-001..004` worked examples (`parse`
declare→parse→query lifecycle, `optional` glued-only value, `enable_help` opt-in +
required-bypass, `required` parse-failure error idiom): def tests in
`arguments/tests/02-worked-examples.loft`, the four `// Example:` citations in
`arguments/src/arguments.loft`, and the generated repo-root `examples-index.tsv`.
Both per-library validations pass: green in library mode
(`EXAMPLES_REPO_ROOT=../loft-libs-core EXAMPLES_CITE_ROOTS="arguments/src
arguments/tests"` — the same two env values `library-ci-reusable.yml` sets), and
deleting the demonstrator file makes all four citations `dangling`. `loft test` and
`loft test --native` are both 17/17. The other five packages in that monorepo
(`cbor`, `crypto`, `random`, `regex`, `zttext`) run the same CI step **vacuously
green** — the opt-in ratchet working as designed, so one library adopting the
convention cannot redden its neighbours.

**`loft-libs-net` is READY TO PR — the first complete repo under the convention.**
`make examples-progress REPO=../loft-libs-net` reads *3 tagged, 1 exempt, 0 deferred,
0 todo* on branch **`worked-examples`** (the plan's no-host-prefix name; pushed, PR
not opened — that needs an explicit ask). Tier 2 (`crypto`, `server`, `web`) is
complete, and finishing the repo took `ssh` and `game_protocol` with it.

**`game_protocol` is the convention's first `exempt`, and it is the row that proves
the verdict column earns its keep.** Every public function is
`msg_<kind>(args) -> GameEnvelope`, the arguments landing verbatim in named fields of
a struct literal; no call order, no encoding, no return value that can mean something
other than it says — its own tests are field-echo assertions, which is what an API a
signature already explains looks like. Untagged, it is indistinguishable from a
package nobody opened; the recorded reason is the whole difference, and it lives in
`examples-exempt.tsv` beside the code rather than in a plan doc nobody reads at
review time.

`ssh` (`@SSH-001..002`): every method self-guards, so a refused connection still
answers a `Session` from which `login`/`open_shell` answer false, `recv` answers "",
and `send`/`resize`/`close` do nothing — right for a terminal app that must not die
mid-frame, but a caller who skips `ok()` runs the entire session ladder, sends every
keystroke, reads an endless empty stream and is never told @SSH-001. And `recv` is a
raw BYTE stream: bound the loop with `size`, never `len`, because `byte_at` indexes
bytes while `len` counts characters, so a `len`-bounded loop drops the tail of every
multi-byte sequence — silently, since both are integers and the loop simply ends
early @SSH-002.

`server` (`@SRV-001..003`): `listen` HALTS on a lost port and that is the design —
a `Server` built on a failed bind is indistinguishable from a healthy one at every
call site, so the process reports itself up and serves nobody, invisibly from
outside too, because a readiness probe reaches whoever DID win the bind;
`try_listen` + `bound()` is the recoverable door @SRV-001 (tagged on the smoke test
that already demonstrated it, rather than writing a second). `header` folds case —
the client picks its own, so a hand-rolled `starts_with("Origin:")` misses the next
one — and rejoins a value's own colons, which a hand-rolled `split(':')[1]` truncates
off an Origin's port @SRV-002. And the return value **says who answered**:
`serve_range` declines with false when there is no `Range` header, the only case the
caller must handle (ignore it and the client waits for a reply nobody sent, or gets
two), while `serve_data` takes every branch and is therefore always true and always
final @SRV-003.

`web` (`@WEB-001..003`): the `pack_*` builder exists because interpolation cannot
carry a zero byte — codepoint 0 is the `character` null sentinel, so `"{a}{b}{c}"`
silently drops every NUL and a binary header with a small integer field is mostly
NULs; the example packs `01 00 02` and shows the interpolated form measuring **2**
bytes, and pins that `pack_take` MOVES @WEB-001. An out-of-range `byte_at` answers
`-1`, which matters *because* 0 is a byte the buffer legitimately carries: `== 0` as
a bounds test reads a real NUL field as end-of-frame **and** an overrun as a NUL
field, wrong in both directions @WEB-002. And a handle is not a connection —
`ws_handler` answers null for a malformed URL only, so a well-formed URL with nobody
listening still yields a usable handle (reconnecting is the library's job) and a
caller reading non-null as "connected" reports itself up against a server that does
not exist; `send`'s false is what reports the link down @WEB-003.

All six run with **no network**, like the rest of those packages' CI tests. @WEB-003
is the pleasing case: its contract is that a down server is *not* an error, so having
no server is exactly what makes it demonstrable. @SRV-002/003 build a `Request` by
hand rather than accepting one off a socket, because what they pin is a DECISION the
request layer makes — which header matched, whether the helper already answered — not
the bytes that reach a client; that is the half a caller gets wrong and the half a
single-process test can hold honestly.

**Stale docs caught in the very functions being documented — one per library so far,
four for four.** `crypto`'s README table called `sha256_b64`'s return a hex digest
when it is base64. `web`'s `tests/byte_at.loft` header said out-of-range returns `0`
when it returns `-1` — and `0` is the one wrong answer to state there, since it is a
legitimate byte value. And `ssh` shipped the bug in its own **documented idiom**: the
header example and `recv`'s doc-comment both wrote `for i in 0..len(out) { byte_at(i,
out) }`, a byte-indexed loop bounded by a CHARACTER count — the exact
`strict-index-text` shape loft lints for, in the library whose whole payload is a
byte stream. All fixed, each pinned by the assertion that found it. The mechanism is
worth naming: writing a worked example forces a value to be *pinned*, and pinning it
is what exposes prose that had drifted. This is the monthly review's failure mode
(a doc that still resolves but no longer matches) arriving at the cheapest possible
moment — and at four for four it is not incidental, it is the second thing the
convention buys.

**Noted coverage gap:** `server`'s multi-client event model (`run` / `poll_event`,
the mutually-exclusive `WsEvent` flags, the pre-split `<msg_id>:<payload>` wire form,
and disconnects absorbed **silently** — the surprising one) owes an example and has
no demonstrator that CI runs; a real one needs a concurrent client+server, which the
package's own smoke test already records as out of reach today. Deliberately left
untagged rather than tagged against a test that does not exercise it.

Third **`crypto` DONE** — the second package on `loft-libs-core`'s
`mac-worked-examples` branch (pushed, PR not opened). Six `@CRY-001..006` worked
examples in `crypto/tests/worked-examples.loft`, cited from the twelve functions
they document. The reason this library owes them is one fact about its surface:
**every parameter and return is `text`**, so the type checker cannot tell a message
from the base64 of a message, a hex digest from a base64 one, a 32-byte seed from a
64-byte key, or a `""` that means *it failed* from a `""` that means *it was empty*.
The six: the two hashing doors (`sha256` text-in/HEX-out vs `sha256_b64` base64 both
ways — and only the base64 door can see a non-UTF-8 byte string at all) @CRY-001;
sign `base64_encode(msg)`, never the message — `"test"` is *itself* valid base64, so
the bare text signs three other bytes **and verifies against itself**, which is why
the mistake is invisible inside one program and only a peer ever sees it @CRY-002;
ES256 determinism (RFC 6979 — two signatures differing is a bug, not entropy) plus
the flat JOSE encodings (bare scalar, `x || y` with no `0x04`, `r || s` not DER), so
check lengths 32/64/64 @CRY-003; a DH output is not a key — run it through HKDF,
where `info` is what separates two keys from one shared secret, and the one-shot is
exactly Extract-then-Expand @CRY-004; the 16-byte tag rides *inside* the sealed blob
and `""` from `open` means AUTHENTICATION FAILED — the same `""` an authentic empty
plaintext answers @CRY-005; `enc` is fresh per seal and must travel with the
ciphertext, while `info` AND `aad` must both match and neither failure says which
@CRY-006. Both validations pass (green in library mode; removing the demonstrator
dangles all six, exit 1), `loft test` / `loft test --native` are 78/78, and a
flipped expected value goes red. The pass also caught a real doc defect in the
function @CRY-001 documents: the README's API table called `sha256_b64`'s return a
hex digest when it is base64 — fixed in its own commit. `loft-libs-core` now reads
2 tagged / 4 TODO (`cbor`, `random`, `regex`, `zttext`).

Second distributed library **`gridmesh` DONE** — pushed as `loft-libs-graphics`
branch `tuxedo-worked-examples` (PR not opened). Five `@GRM-001..005` worked
examples in `gridmesh/tests/worked-examples.loft`, cited from the seven functions
they document: the edit→collect→rebuild→**clear** cycle (@GRM-001), `cell_ixs`
(emit) vs `halo_ixs` (read-only, owned by the neighbour) (@GRM-002), cell indices
surviving a removal uncompacted while a repaint gets a fresh one (@GRM-003), a
dirty GROUP listing its CLEAN member chunks so their cached meshes are copied
(@GRM-004), and the `step_x`/`step_y` parity pair + `idx_at`'s `-1`-not-null
sentinel (@GRM-005). What a toolkit owes an example is the SHAPE of a correct
call, not a single signature — every one of the five is a contract a caller can
get wrong while type-checking. Both validations pass (gate green in library mode;
removing the demonstrator dangles all five, gate exit 1) and `loft test` /
`loft test --native` are 25/25; a flipped expected value goes red, so neither
channel is vacuous. `hex_grid` (the other tier-1 library) is **held**: its
monorepo `loft-libs-world` has uncommitted work in the tree, and the rollout's
one gate is a clean, current checkout.

**What "clean" has to mean, refined by `loft-libs-net`.** That repo's tree was not
pristine either — but every entry was `.loft/` build residue, including one *tracked*
cache file (`server/tests/.loft/cache/…`) that a test run had deleted. Build cache is
not someone's work, so the rollout proceeded there and simply never staged those
paths; `loft-libs-world`'s modified `README.md` and lock files are a different thing
and the hold stands. The distinguishing question is not `git status` being empty, it
is whether a human or another agent would lose anything. (`loft-libs-core` fixed its
own version of this by ignoring `.loft/` by directory NAME rather than one row per
place someone noticed — `loft-libs-net` still tracks a cache file and wants the same
one-line fix, filed here rather than folded into a docs branch.)

Mechanism complete: Phase A (probe), Phase B foundation + **acronym registry
broadened** to the distributed monorepos, Phase C indexer ingestion, and the
**shared gate made repo-agnostic + run from `library-ci-reusable.yml`** so a
`loft-libs-*` repo's own citations are validated in its CI with no per-repo copy
(loft self-check byte-identical, synthetic-lib probe green/dangling/duplicate). Tagged so far:
`@STD-001..012` (stdlib), `@GIT-001..005`, `@LEX-001..002`, `@ACR-001..003`,
`@EHK-001..004` (in-tree libraries), `@ARG-001..004` + `@CRY-001..006`
(`loft-libs-core`), `@GRM-001..005` (`loft-libs-graphics`), `@SRV-001..003` +
`@WEB-001..003` + `@SSH-001..002` (`loft-libs-net`). **The distributed libraries are this stream's
to roll out** — they are shared code with their own validated contract (each
`library-ci.yml` + the register's recorded `api`), not a per-agent private tree, so
loft authors their tags in the canonical monorepo (per `loft-registry/index.json`;
work only in a clean, current checkout). Tier 1 is done except `hex_grid`
(held — dirty monorepo); next: tier 2, `crypto`/`server`/`web`. The genuine wait-for-their-agent case is
only the first-class *apps* (`dryopea`/`crawler`/`moros`), which loft merely cites.

The loft **stdlib** was the starting library
(source 3, in-repo, loft's own gate — the cleanest arrow): twelve functions across
four clusters carry `// Example: @STD-0NN` citations — text (`starts_with_at`,
`chr`, `join`), collection aggregates (`min_of`, `max_of`, `sum`, `tree_walk`), JSON
(`json_parse` navigation @STD-007, the `json_array`/`json_object` builders @STD-008,
the `struct_from_jsonvalue`/`struct_to_json` round-trip @STD-009), and files/IO (the
missing-vs-empty `content`/`read_bytes`/`list_dir` `T?` contract @STD-010, `lines()`
CRLF-normalisation @STD-011, the `FileResult`/`ok()` classify idiom @STD-012) —
resolving to tagged tests (two examples reuse existing clear tests —
`445-generic-tree-walk.loft` for `tree_walk`, `562-file-read-missing-null.loft` for
the null contract — the rest live in `945-stdlib-worked-examples.loft` and the
filesystem-isolated `946-stdlib-file-worked-examples.loft`, all green on both
backends). The gate is `scripts/check_doc_drift.sh examples` (dangling + duplicate),
wired into the `all` run that CI already blocks on — proven red on both a dangling
citation and a duplicate tag. All four stdlib clusters are covered.

**Cross-repo resolution now works (Phase B + first-class-app source).** The acronym
registry lives at `scripts/example_repos.tsv` (acronym → repo → git url → branch).
Projects are assumed checked out as **siblings** in one parent dir, so a repo's
checkout is simply `../<repo>` (no per-entry path navigation; the current repo is
scanned in place). A `// Example: @AAA-###` citation is resolved by acronym:
- the owning repo's tag may sit above **any `fn`** — a `test_*` OR a real function in
  a first-class application's own source (dryopea tags both `tests/` and `src/`);
- a repo with a **local checkout is validated offline against it** (preferred) and
  the check emits the real git blob link (verified live: `@DRY-001` →
  `dryopea/blob/main/tests/25_m3_the_ground_gl.loft#L43`);
- a cross-repo tag whose repo is **not** checked out is reported `unvalidated` (a
  warning, not a hard failure — keeps CI deterministic) with the link still emitted.

Follow-up: hard *online* validation without a checkout (fetch a published per-repo
tag index) — today the offline local-checkout path is the validated one.

## Goal

For a library function whose **correct use is not obvious from its signature +
doc**, point the reader at a **real call site** instead of a prose snippet. Three
sources, in priority order:

1. **The library's own tests** — a test that already demonstrates the function,
   made findable by a stable tag.
2. **A real use in a first-class application** (`moros`, `dryopea`, `crawler`) or
   its tests — the strongest example, because it is the function doing a real job.
3. **A loft in-repo demo application or example program** — `examples/*.loft` and
   the `gallery` / `game` / `play` / `crystal-editor` / `native-editor` demos. For a
   **stdlib function or a language feature** this is often the best *and* cleanest
   source: it is a real program a user runs, and it lives in the loft tree itself, so
   it resolves through loft's own gate with **no cross-repo arrow** (unlike source 2).

The same applies to the **loft feature catalogue** (`@F`/`@I` issues): a language
feature also wants a tag pointing at how it is used **in real life**, on top of the
synthetic run-snippet the catalogue already generates (Phase C2).

The example is a *pointer to working code*, never a snippet in a comment: a snippet
rots silently at the first signature change; a tagged test cannot drift from working
code because it **is** working code (dryopea `EXAMPLES.md § Why not a snippet`).

**Scope discipline (the heart of this plan).** Tag a function **only where a real
call site teaches more than its signature does**. A one-line accessor, a function
whose usage is self-evident from its type — no example; the doc already suffices.
No retroactive sweep of every `pub fn`. This mirrors dryopea's opt-in ratchet, and
is what keeps the gate from going red on hundreds of trivially-obvious functions the
day it lands.

**When no clear example exists yet, write one — in retrospect.** The two sources
above may need to be **created**, not merely cited: if a non-obvious function or
feature has no test that shows it used *correctly*, authoring that test is part of
the deliverable (dryopea `EXAMPLES.md` rule 4 — write a new test if none of the
existing ones is CLEAR). The signal for *which* functions owe one is concrete, and
is the motivation for this plan: **a reader — often an AI agent — that knows a
function/feature exists but cannot use it effectively from its doc alone, and only
succeeds once pointed at an application that already uses it, is exactly the case
that owes a worked example.** Lift the pattern from that real usage into a tagged
test, so the next reader never needs the pointer. This is not a contradiction of the
ratchet: the ratchet says don't tag the *obvious*; this says for the *non-obvious*,
a missing example is a gap to fill, not a reason to skip.

## The mechanism (as built — adapted from dryopea)

- **Tag family `@AAA-###`** — `@`, three-letter acronym, hyphen, three digits
  (`@STD-001`). Distinct from loft's `@P`/`@PLN`/`@F`/`@GH` families (none has that
  hyphen). One acronym per repo, claimed **ecosystem-globally**.
- **Citation** — `// Example: @STD-001` on a comment line directly above the
  function it documents (stdlib fn in `default/`, library fn in `lib/`).
- **Definition** — the tag sits in a comment block directly above the `fn` it names
  (a blank line breaks the block). The `fn` may be a `test_*` **or a real function
  in a first-class application's own source** — a live use documents as well as a
  test does.
- **Acronym registry** — `scripts/example_repos.tsv`: `acronym → repo → git_url →
  branch`. Projects are assumed checked out as **siblings** in one parent dir, so a
  repo's checkout is `../<repo>`.
- **Gate** — `scripts/check_doc_drift.sh examples` (part of the `all` run CI's
  doc-hygiene job already blocks on). Faults: `dangling` (cited, no fn carries it),
  `duplicate` (one tag on two fns in this repo), `unregistered` (acronym not in the
  registry) — all drift; `unvalidated` (cross-repo tag whose sibling isn't checked
  out) — warning only, link still emitted. Proven red on dangling + duplicate.
  **The FIRST tag in a definition block defines it.** An example's prose routinely
  names a sibling ("the failure path, see @ARG-004"), and letting a later mention win
  read that block as defining the *sibling* — surfacing as a `dangling` on the block's
  own tag plus a `duplicate` on the one it mentioned, both pointing away from the real
  mistake (found on the `arguments` rollout, the first library whose examples
  cross-reference each other). Verified a no-op on every existing tree: the def scan is
  byte-identical over loft, `loft-libs-graphics`, `loft-libs-net` and `dryopea`.
  *Not yet built:* `orphan`, opt-in `uncovered`, and a self-test harness — deferred
  (the non-vacuous set of real @STD citations is what keeps the green honest today).

### Design decision — which way the cross-repo arrow points

A library must **not** gain a code dependency on its consumers, and a first-class
app is not a dep in the library's `loft.toml`. So the two sources resolve
differently, and this split is load-bearing:

| source | tag DEFINED in | citation lives on | resolved by (as built) |
|---|---|---|---|
| library's own test | this repo's `tests/` | the fn's doc comment | `check_doc_drift.sh examples`, scanned in place |
| loft in-repo demo / example program | this repo's `examples/` / `tools/` | the stdlib fn / feature issue | same — same tree, no cross-repo arrow |
| real use in a first-class app | the app's `tests/` **or its `src/`** | the stdlib / library fn | the registry maps the acronym to `../<repo>`; validated offline against that sibling checkout, git link emitted |

The gate only ever checks a citation against the repo its **acronym** names, and a
foreign repo is consulted **read-only** via its sibling checkout — never a build
edge, so every library still builds and tests standalone. A cross-repo tag whose
sibling isn't checked out is a warning, not a failure, so CI stays deterministic.
The demo-program row is the case that needs no foreign repo at all: a loft
`examples/` program documenting a stdlib function or feature is in the **same tree**
as what it documents — the preferred source for stdlib + feature examples.

## Phases

Cut per the two-bounds rule (each can go red on its own, for a real reason).

### Phase A — Probe: does the cross-repo pointer work, and does it stay inert? — DONE

Done differently than first sketched (no `arguments`/`examples.sh` port): the probe
rode the real gate. A temporary `// Example: @DRY-001` in loft's stdlib **validated
against the sibling `../dryopea` checkout** and emitted the git blob link; `@DRY-999`
went `dangling`; `@MOR-001` (moros not checked out) stayed an `unvalidated` warning
without failing. Confirms all three: the cross-repo pointer resolves, it produces a
real link, and it is inert (read-only, never a build edge) when the foreign repo is
absent.

### Phase B — The acronym registry — DONE (foundation), still to broaden

Built as `scripts/example_repos.tsv` (a loft-repo file for now, not yet the
`loft-registry` shared home — see Open questions). loft's own: `STD` · `GIT`
(in-tree `lib/git`) · `LEX` (in-tree `lib/lexer`) · `ACR` (in-tree
`lib/audience_crystal`) · `EHK` (in-tree `lib/engine_host`) — one repo may own
several acronyms. First-class apps: `DRY` dryopea · `CRW` crawler · `MOR` moros.

**Distributed libraries registered (claim staked, tags not authored yet).** With
the sibling monorepos pulled, the library acronyms are entered — mapped to their
**monorepo** checkout, not a per-package repo (the design the checkout layout
forced, below): `loft-libs-core` → `ARG` arguments · `CRY` crypto · `RND` random ·
`loft-libs-graphics` → `GFX` graphics · `GRM` gridmesh · `SHP` shapes ·
`loft-libs-net` → `GMP` game_protocol · `SRV` server · `WEB` web ·
`loft-libs-world` → `HXG` hex_grid · `HXT` hex_terrain · `HXW` hex_world. These
rows **stake the ecosystem-global acronym**; none of the monorepos carries a
worked-example tag yet — except `ARG`, whose four are now authored (Status). Still to
add when reached: `MKD` markdown (no sibling checkout found — deferred), the newer
`imaging`/`ssh`/`cbor`/`regex`/`zttext` packages, and `FTR` the feature catalogue
(Phase C2). An acronym is only needed the moment a package authors its first tag: the
per-package CI step is vacuously green without one, so an unregistered package cannot
go red before it opts in.

**These libraries ARE this stream's to roll out** — they are shared code between
agents (not one consumer-agent's private tree, the way dryopea/crawler/moros are),
each guarded by its **own validated contract** (its `library-ci.yml` gate + the API
signatures recorded in the register). So the `edit-only-this-repo` dogfood rule that
holds for a first-class *app* does **not** apply here: this stream authors the
libraries' worked-example tags directly, in the library's canonical location. The
one operational caveat is ordinary git-safety — work only in a **clean, current**
checkout (`loft-libs-graphics`/`-net` were clean when surveyed; `loft-libs-world`/
`-core` were dirty/behind, so leave those until clean rather than risk a concurrent
edit).

- **Design decision — monorepo, not per-package (registry `repo` = the monorepo).**
  The registry's `../<repo>` assumption was written for one repo per package
  (`../gridmesh`). Reality is monorepos: `gridmesh` lives at
  `../loft-libs-graphics/gridmesh`. So the `repo` column names the **monorepo** and
  several acronyms map to it. This needs **no gate change**: `examples_defs_in_tree`
  scans the whole monorepo for the acronym's tag and captures a **repo-relative**
  path, so the emitted git link is correct
  (`…/loft-libs-graphics/blob/main/gridmesh/tests/x.loft#L12`).
- **Canonical location = the monorepo subdir, per the register.** The authority for
  where a library lives is `loft-registry/index.json`: each package entry carries a
  `homepage` (`…/loft-libs-graphics/tree/main/gridmesh`) and every version a
  `subpath` (`gridmesh`) + release `url` on the monorepo. So a worked-example tag is
  authored in that canonical tree — never in loft's `lib/<name>` stubs (empty
  scaffolding) nor an installed/`~/.loft/` copy (a lagging cache). The register also
  records each version's `api` signatures — the contract the tag documents against.
- **Still to do:** a duplicate-**acronym** guard (two repos claiming one acronym) —
  the gate today guards duplicate *tags* within a repo, not acronym collisions across
  the registry.

### Phase C — Shared gate + indexer ingestion — PARTIAL

- **Done:** the gate is one shared script — `scripts/check_doc_drift.sh examples` —
  not a per-library copy; it already resolves cross-repo citations and **emits the
  git link** (the "carry a real link" half of what the indexer owes).
- **Done:** loft's tag indexer (`make index` / `scripts/idx`) now ingests
  `@AAA-###`, so `scripts/idx tag:@STD-011` resolves both the citation and the
  demonstrating fn. Added one branch to `tools/indexer/src/scan.loft`'s per-byte
  `scan_line` dispatch (three uppercase letters, hyphen, three digits; the interior
  hyphen disambiguates from `@P`/`@PLAN`/`@F`/`@GH`, and it is checked AFTER `@F`/`@I`
  so `@FTR-001` is not eaten by the `@F` branch). `tag_is_valid` already accepts the
  shape via its fallthrough, so `idx broken` stays `[]`; the query side (`scripts/idx
  tag:…`) is a generic bucket lookup and needed no change. Verified: all 16
  `@AAA`-shaped tags in the tree index (12 `@STD` + the plan's `@DRY`/`@MOR`/`@FTR`
  mechanism examples), no accidental matches, `idx broken` + `broken-links` clean.
- **Still to do (Follow-up from Status):** hard *online* validation of a cross-repo
  tag when its sibling isn't checked out (fetch a published per-repo tag index),
  crawling a `~/.loft/`-style hidden root correctly (dryopea's traversal-from-root
  bug — a `--exclude-dir='.*'` scan reads zero files under any hidden path).

### Phase C2 — Worked examples for the feature catalogue (S)

The same mechanism applies to the **feature catalogue** (`loft-lang/features`,
`@F<N>` feature / `@I<N>` infra), not just library functions — a reader learning a
language *feature* is best served by seeing it used **in real life**, not only by a
synthetic snippet.

- **What exists today:** the generator promotes the **first** ` ```loft ` fence in a
  feature issue into a RUN test (`tests/docs/features/*.loft`). That proves the
  feature compiles and runs, but it is a teaching snippet — it does not show the
  feature carrying real work.
- **What this adds:** a feature issue gains an `Example:` citation to a `@AAA-###`
  tag naming a **real** test or first-class-app usage that exercises the feature.
  Same tag family, same indexer (Phase C), same `dangling`/`orphan` gate. The two
  are complementary and both kept: the first-fence RUN test is the minimal
  compiles-and-runs gate; the tag is the pointer to idiomatic real-life use. The
  **preferred source for a feature** is a loft in-repo demo/example program (source 3
  above) — same tree, no cross-repo arrow. Where a feature has no test or demo
  showing real use, **author one in retrospect** — the same rule as the library
  rollout.
- **Where the citation lives:** in the **issue body** (canonical), never in the
  generated shadow (`index/features.json`, `doc/features/`, `tests/docs/features/`)
  — the `features-check` drift guard fails on hand-edits. Edit the issue, then
  `make features-fetch && make features-gen`.
- **Acronym:** the catalogue gets its own — `FTR` — registered in Phase B alongside
  the libraries', since a feature tag shares the ecosystem-global namespace.
- **Validation (can go red):** `make features-check` stays green; a feature citing a
  deleted test goes `dangling` through the same gate; `idx tag:@FTR-001` resolves
  the real-usage test.

### Phases D… — Per-library rollout, one library per phase (S each)

**Started with the loft stdlib** (`@STD`, source 3, in-repo): text
(`starts_with_at`, `chr`, `join`, @STD-001..003), collection aggregates
(`min_of`/`max_of` @STD-004, `sum` @STD-005, `tree_walk` @STD-006 — the last reusing
an existing test), JSON (`json_parse` navigation @STD-007, the
`json_array`/`json_object` builders @STD-008, the struct↔JSON round-trip @STD-009),
and files/IO (the missing-vs-empty `content`/`read_bytes`/`list_dir` contract
@STD-010 — reusing `562-file-read-missing-null.loft` — `lines()` CRLF-normalisation
@STD-011, the `FileResult`/`ok()` classify idiom @STD-012). The four stdlib clusters
are covered; the rollout moves to the registered libraries below.

**In-tree libraries shipping.** Each anchors its tags on **real, exercised code** —
a live consumer or a CI-run test — never a prose snippet:
- **`lib/git`** (`@GIT-001..005`, registered `GIT`→loft): tags on live uses in two
  consumers run every `make index` / `make view` — `scan.loft` (`tracked_files`
  @GIT-001) and `refresh.loft` (`ahead_behind` @GIT-002, `log`-vs-`head`-date
  @GIT-003, `changed` rename→new-path @GIT-004, `numstat` `-1`-is-binary @GIT-005).
- **`lib/lexer`** (`@LEX-001..002`, `LEX`→loft): the core cursor idiom — `matches`
  (consume-if-equal) vs `test` (peek) + `identifier` — tagged on `parser.loft`'s
  `function` grammar rule (@LEX-001), plus the **backtracking protocol**
  `anchor`/`revert` (@LEX-002) tagged on `parser.loft`'s `object` rule, which
  anchors the cursor, attempts an object literal, and rewinds on the first
  non-matching field. Both are exercised by the `16-parser` doc test (which parses
  a function and an object literal). (The format-protocol / comment functions owe
  examples too, but their only current demo is the *rendered* `15-lexer` doc test,
  which must not carry a tag — a noted coverage gap for a non-rendered demo.)
- **`lib/audience_crystal`** (`@ACR-001..003`, `ACR`→loft): tagged on the
  `01-editor-helpers` test (run both backends by `wrap.rs`) — `hex_at_world`
  picking inverse @ACR-001, the `crystal_incr_new`/paint/rebuild/`full_mesh` +
  `crystal_mesh_to_lines` stride-10 loop @ACR-002, `crystal_incr_erase`
  edit-then-rebuild @ACR-003.
- **`lib/engine_host`** (`@EHK-001..004`, `EHK`→loft): the @PLN18 server kernel,
  a sequence-sensitive API — tagged on the audience-demo kernels that CI spawns
  (`engine_host_audience.rs`, `engine_host_udp.rs`): `run` loop-skeleton with
  closures-as-args @EHK-001, `broadcast`-to-all vs `send`-to-one @EHK-002, the
  `fast_lane`/`fast_lane_keyed` sync-lane declaration *before* `run` @EHK-003, and
  `run_client` + the `client_sync_next`/`client_sync_payload` drain loop @EHK-004.

All resolve through `idx` and the gate; each host program / test still runs.

**All the distributed libraries are this stream's to roll out.** They are shared
code between agents, each guarded by its own validated contract (its `library-ci.yml`
gate + the register's recorded `api`), so — unlike a first-class *app* — this stream
authors their worked-example tags directly, in the canonical monorepo tree (per the
register; see Phase B). The in-tree libraries went first only because they were the
nearest to hand (`lib/git` — done — anchoring live uses in `scan.loft` +
`refresh.loft`, no test authored); the priority order below then covers the
distributed monorepos. The one gate is git-safety: roll out only in a **clean,
current** checkout, and leave a dirty/behind monorepo (e.g. `loft-libs-world` was
39 behind) until it is clean. The genuine wait-for-their-agent case is narrower than
first written: it is the first-class *apps* (`dryopea`/`crawler`/`moros`), whose own
agents author their `@DRY`/`@CRW`/`@MOR` tags — loft only *cites* those.

One library per phase (the skill's one-call-site-at-a-time shape). For each: tag the
highest-value functions from its own tests and, where it exists, a real consumer
usage — and where **no** clear demonstrating test exists, **author one in retrospect**
(lifting the pattern from a first-class app that already uses the function correctly)
rather than skipping the function; the shared `check_doc_drift.sh examples` gate
covers it (no per-library `examples.sh` to wire).
- **Validation per library:** gate green on that library **and** a deliberately
  deleted test makes a citation dangle.
- **Priority order** (non-obvious usage first, where a real call site pays most):
  1. `arguments` (DONE, `@ARG-001..004`), `gridmesh` (DONE, `@GRM-001..005`),
     `hex_grid` (HELD — `loft-libs-world` has uncommitted work in its tree) —
     stateful/geometry APIs where the shape of a correct call is the whole
     question. Confirmed by the two done: every tag is a contract a caller can get
     wrong while type-checking (a forgotten `clear_dirty`, a halo cell emitted
     twice, a stale cell index, a group rebuilt whole, a `-1` read as null).
  2. **DONE** — `crypto` (`@CRY-001..006`), `server` (`@SRV-001..003`), `web`
     (`@WEB-001..003`): protocol/sequence APIs where the order of calls matters.
     The tier sharpened what "gettable wrong while type-checking" means. For
     `crypto` the whole surface is `text -> text`, so the *encoding* is the
     contract and the compiler is blind to all of it. For `server` and `web` it is
     the RETURN VALUE that carries the contract — `serve_range`'s false, `send`'s
     false, `byte_at`'s `-1`, a non-null `WsHandler` — each a well-typed answer
     that means something other than what a caller assumes.
  3. `markdown`, `graphics`, `random`, `shapes`, `game_protocol` — as they're touched.
  4. `hex_terrain`, `hex_world`, in-repo `moros_*` — opportunistic (opt in when a file
     is finished; the ratchet only goes up).

  Off the priority list but done, because finishing a REPO is what earns a PR:
  `ssh` (`@SSH-001..002`) and `game_protocol` (**exempt**), which together closed
  `loft-libs-net`. Priority orders which library pays most; the PR unit decides which
  ones you actually finish first.

### Landing the rollout — one branch per repo, PR when the repo is complete

**The PR unit is the REPO, not the library.** A per-library PR buys nothing: the
gate is opt-in, so a half-adopted repo is already green, and each PR costs a full
CI cycle to land a doc-comment and a test file. Batching by repo is the same
bundle rule that governs plan closes, and it makes the PR mean something a
reviewer can judge — *this repo has adopted the convention* — instead of
*one more function got a comment*.

**One branch per repo, continued across sessions and hosts.** If a rollout branch
already exists, commit the next package onto **it** — never fork a second. A new
one is named `worked-examples`, with **no host prefix**: the branch belongs to the
work, not to the agent, and a host prefix is exactly what produces two
half-finished rollout branches in one repo when the next session runs on another
machine. (The two in flight predate this and keep their names:
`mac-worked-examples` in `loft-libs-core`, `tuxedo-worked-examples` in
`loft-libs-graphics`.) **Rebase onto `origin/main` before each package's commit** —
these repos publish from `main` while the rollout runs, so one package's worth of
divergence is the most that should ever need reconciling.

**Done, defined so it can be checked.** Every package (a directory with a
`loft.toml`) must carry one of three verdicts:

| verdict | recorded where | means |
|---|---|---|
| **tagged** | the generated `examples-index.tsv` lists ≥1 tag under `<pkg>/` | it has its worked examples |
| **exempt** | `examples-exempt.tsv` (repo root, hand-written) + a reason | no function here teaches more from a call site than from its signature — the convention's own no-retroactive-sweep clause, made explicit |
| **deferred** | `examples-exempt.tsv` + a reason naming what unblocks it | one does owe an example, not in this pass; it returns via the monthly by-hand review |

**Silence is not a verdict.** An untagged, unlisted package is TODO and holds the
PR — which is the whole point: without a recorded *exempt*, "all libs done" would
be indistinguishable from "nobody looked at the rest", and the rollout would ship
a repo whose quiet packages had never been judged. Zero TODO ⇒ open the PR.

```bash
make examples-progress REPO=../loft-libs-graphics   # run it ON the rollout branch
```

The report reads the working tree, so on `main` a package whose tags live on the
rollout branch reads TODO — check out the branch first. It **exits 0 always, is
not part of `all`, and library CI never runs it**: a half-adopted repo must stay
green, because the opt-in ratchet is what lets the rollout proceed one package at
a time without reddening a neighbour.

**Completion is frozen to the packages present when the branch opens.** A package
added mid-rollout adopts later through the same ratchet. Without that, a 14-package
repo (`loft-libs-world`) is a moving target that never converges.

**Where that leaves the three branches in flight** (`make examples-progress`):

| repo | branch | state |
|---|---|---|
| `loft-libs-net` | `worked-examples` | **READY TO PR** — 3 tagged (`server`, `ssh`, `web`), 1 exempt (`game_protocol`), 0 todo |
| `loft-libs-core` | `mac-worked-examples` | 2 tagged (`arguments`, `crypto`) / 4 todo (`cbor`, `random`, `regex`, `zttext`) |
| `loft-libs-graphics` | `tuxedo-worked-examples` | 1 tagged (`gridmesh`) / 3 todo (`graphics`, `imaging`, `shapes`) |

`loft-libs-net` is the convention's first complete repo, which is what makes the
"the PR unit is the REPO" rule reviewable rather than theoretical — a reviewer is
handed *this repo has adopted the convention*, with a recorded reason for the one
package that has nothing to demonstrate. **Opening the PR needs an explicit ask**
(the branch is pushed and the report is green; the plan's "zero TODO ⇒ open the PR"
sets the readiness bar, not the permission).

### Phase (last) — Convention doc + CI ratchet (S) — CI RATCHET DONE

**The shared gate now runs in every library's CI, from a single source.** The
distribution question had an answer already in the tree: every `loft-libs-*` repo's
`library-ci.yml` is a thin caller of the reusable workflow
`loft-lang/loft/.github/workflows/library-ci-reusable.yml@main`, which checks out
the library at `$GITHUB_WORKSPACE` **and** loft into `loft-src/`. So the shared gate
(`loft-src/scripts/check_doc_drift.sh`) and the acronym registry
(`scripts/example_repos.tsv`) are ALREADY present in every library CI run — "libraries
share it, not a per-repo copy" is satisfied natively, with no `loft-registry` migration.

Built (loft-side only, no library-repo edit):
- `check_examples` made **repo-agnostic** via three env knobs that all default to
  loft's own self-check byte-for-byte: `EXAMPLES_REPO_ROOT` (repo under test, `.`),
  `EXAMPLES_CITE_ROOTS` (dirs grepped for citations, `default lib`), `EXAMPLES_REGISTRY`
  (for test isolation). The script cd's to the loft root at startup, so the registry
  and cross-repo link logic stay loft-anchored while the citation + local-def scan
  follow `REPO_ROOT`. Three repos resolve in place: the repo-under-test (its own
  acronym), the **loft host repo** (its acronyms — `STD`/`GIT`/… — always available at
  `.`, since the gate runs from inside loft even when loft is checked out as `loft-src`
  in a library CI, so a library citing a loft `@STD` tag hard-validates to the loft
  blob link, and a missing one is real `dangling` drift), and a foreign sibling
  checkout `../<repo>` (an `unvalidated` warning only when genuinely absent, e.g. an
  app repo like `moros`).
- `library-ci-reusable.yml` gained a **gating** per-package step running that gate with
  `REPO_ROOT=$GITHUB_WORKSPACE` + `CITE_ROOTS="<pkg>/src <pkg>/tests"`. **Vacuously
  green for a package with no citations**, so it is safe on all libraries from day one
  and only begins gating once a package opts in by authoring a citation — the ratchet,
  enforced by construction rather than by a switch-off-within-a-week promise.

Validated: loft's own `check_doc_drift.sh examples` output is byte-identical before/after
(no self-check regression), and a synthetic library tree proved `ok` / `dangling` /
`duplicate` / `unregistered` all fire correctly in library mode.

**Published per-repo index — where each tag lives, without a checkout.** Each repo
carries a generated `examples-index.tsv` at its root: one row per `@AAA-###`-tagged fn
— `tag ⇥ file:line ⇥ fn ⇥ git-blob-link` — so a reader (or loft's cross-repo `idx`)
learns where a tag resolves without cloning. It is GENERATED, never hand-edited:
`make examples-index` (or `scripts/check_doc_drift.sh write-examples-index`) writes it;
`check_doc_drift.sh examples-index` VERIFIES the committed copy is current (fail-on-diff,
the `features-check` pattern), wired into both loft's `all` gate and the per-package
library-CI step. Line numbers churn, so the index lives WITH the code it indexes — the
central `example_repos.tsv` stays one stable row per acronym. **Auto-created on commit**
by a `.githooks/pre-commit` hook (regenerates + stages it, so every PR carries a current
index); the CI verify is the backstop for a commit made without the hook installed.
Vacuously "no index needed" for a repo with no tags, so it never reddens a library
before it opts in.

**Convention page — DONE.** The shared page the libraries point at instead of
re-explaining the tag family is [LIBRARY_AUTHORING.md § 2a — Worked examples](../../LIBRARY_AUTHORING.md#2a-worked-examples--point-at-a-real-call-site)
(a section, not a new top-level doc): the scope discipline, the citation/definition
two-halves + acronym registry, and the note that the gate already runs in every
`loft-libs-*` CI via the reusable workflow and is vacuously green until a package opts
in. It cross-links to [LIBRARY_DOC_REVIEW.md](../../LIBRARY_DOC_REVIEW.md) for the
staleness/quality half.

### Monthly by-hand review — the ongoing home (DONE, first pass 2026-08)

The automated `check_doc_drift.sh examples` gate only sees a citation that
*dangles* or *duplicates*. Two failures need a human: a doc that still resolves
yet no longer matches the code (**staleness**), and an example that is valid but
no longer the clearest (**quality**). Those are handled by a monthly by-hand pass
at the release beat — [../../LIBRARY_DOC_REVIEW.md](../../LIBRARY_DOC_REVIEW.md),
driven by `scripts/doc-review.sh` (coverage + citation inventory + a
changed-since-watermark worklist), wired into [RELEASE.md](../../RELEASE.md)'s
monthly cadence. It is a hygiene **ratchet, never a release blocker** — the
watermark bounds each pass to what moved. This is where the per-library rollout's
staleness/quality upkeep lives once a library is tagged.

## Open questions

- ~~**Registry home**~~ — RESOLVED: stays `scripts/example_repos.tsv` in the loft
  repo. A migration to `loft-registry` is unnecessary because the library CI already
  reaches the loft tree — every `loft-libs-*` `library-ci.yml` is a thin caller of the
  reusable workflow in loft, which checks loft out into `loft-src/`. So the single
  copy in loft IS the shared copy every library reads; no second home is needed.
- **Online cross-repo validation** — a sibling checkout validates offline today; a
  repo not checked out only warns. A published per-repo tag index (one small file
  fetched by raw URL) would let it hard-validate offline-of-clone. (Phase C follow-up.)
- **`// Example:` spelling** — kept dryopea's spelling for one shared indexer.
- **In-repo `lib/moros_*`** — first-class-app-internal libs; lower priority than the
  registered/sibling libraries a broad audience consumes.

## See also

- dryopea `docs/EXAMPLES.md` — the origin design + the `examples.sh` gate this adapts.
- dryopea `plans/26-the-fixed-step` — first library shipped under the convention.
- The loft-side gap this plan closes: `scripts/idx` + `make index` recognise
  `@P`/`@PLN`/`@F`/`@GH` today, not `@AAA-###`, and crawl only the loft tree.
- Feature catalogue (Phase C2): `loft-lang/features` issues + the
  `make features-fetch/gen/check` shadow; the generator's first-` ```loft `-fence
  RUN test is the compiles-and-runs gate the real-usage tag complements.
