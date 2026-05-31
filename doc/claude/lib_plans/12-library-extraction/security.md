<\!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Plan-12 — security advisory channel

Part of [@PLAN12 library extraction](README.md).  Covers
**Phase 6.7** — the signed `advisories.json` feed sibling to
`index.json`, typed severity tiers on yanked versions, and
the loft-binary classifier that fail/warns by severity.  Same
schema covers `"package": "loft"` advisories for the
non-drainable stdlib floor.  Pairs with
[lib-plan 30 § Phase 30.4](../future/30-loft-distribution/README.md)
for the binary-side trust chain.

Also covers the **verify-on-recompile** verification-timing
table — every "when do we hash?" decision for both libraries
(here) and the binary (in lib-plan 30).

Companion docs:
- [registry-resolution.md](registry-resolution.md) — `loft
  update` consumes the advisory channel to skip yanked
  versions.
- [offline.md](offline.md) — stale-advisory thresholds for
  air-gapped environments.

---

### Phase 6.7 — security advisory channel (proposed 2026-05-31)

**Trigger.**  6.6 ships auto-install, which makes adoption
broader.  Broader adoption + a YEAR-old cached version + a CVE
filed today = users running known-vulnerable code with no
mechanism for the registry to tell them.  The package format
already has a `yanked` field on each entry, but (a) the schema
is untyped (no severity tier), (b) there's no separate
fast-refresh feed (the full `index.json` is large and only
refreshed periodically), and (c) the loft binary doesn't check
the yank list on every invocation.  This phase closes those
gaps.

**Schema bump.**  Each version entry gains an optional typed
`status`:

```json
"0.1.1": {
  ...,
  "status": {
    "kind": "yanked",
    "severity": "security_critical",
    "advisory": "GHSA-xxxx-yyyy-zzzz",
    "summary": "TLS bypass in ws_client_connect"
  }
}
```

Severity tiers, with default loft-binary behaviour:

| Tier | Behavior |
|---|---|
| `security_critical` | **Refuse to build / run.**  Exit non-zero with the advisory URL.  Override: `LOFT_SECURITY_OVERRIDE=<advisory-id>` (env var, audit-trail). |
| `security_high` | **Warn loudly** at start of every run; non-zero exit only under `--strict-security` (CI flag). |
| `security_low` / `bug` | One-line warning per run. |
| `deprecated` | One-line note per day (suppressed by daily-cadence state). |

**Advisory feed — `advisories.json`.**  Sibling to `index.json`
in the registry, signed by the same Ed25519 key.  Schema:

```json
{
  "schema_version": 1,
  "updated": "2026-05-31T12:00:00Z",
  "retention_days": 90,
  "advisories": [
    {
      "id": "GHSA-xxxx-yyyy-zzzz",
      "packages": [{"name": "web", "affected": ">=0.1.0, <0.1.2", "fixed_in": "0.1.2"}],
      "severity": "security_critical",
      "summary": "TLS bypass in ws_client_connect",
      "published": "2026-05-30T08:00:00Z",
      "references": ["https://github.com/loft-lang/loft-libs-net/security/advisories/..."]
    }
  ]
}
```

Two reasons it's separate from `index.json`:

- **Refresh cadence.**  `advisories.json` is small (~kilobytes;
  90-day retention) → cheap to refresh every 24h on the user's
  loft binary.  `index.json` is the full catalog → refresh every
  7d, batched with cold install.
- **Audit shape.**  Retained advisories are append-only; old
  entries don't churn when a new package version ships.  Easier
  to mirror, monitor, and audit independently of the active
  catalog.

**Loft-binary check.**  On every invocation that resolves a
package (cached install OR fresh auto-install):

1. Compute (package_name, version) tuples for each loaded
   library.
2. Load `~/.loft/registry/advisories.json` (refresh if >24h
   old AND online).
3. For each tuple, check the affected range against advisory
   entries; classify by severity.
4. Apply the severity table above.

**Output examples.**

```
# security_critical — fail
$ loft my_script.loft
error: gridmesh 0.1.1 was yanked for a security vulnerability
  advisory: GHSA-xxxx-yyyy-zzzz
  summary:  TLS bypass in ws_client_connect
  fix:      gridmesh >=0.1.2 (run `loft install gridmesh@0.1.2`)
  override (audit-trail required): LOFT_SECURITY_OVERRIDE=GHSA-xxxx-yyyy-zzzz

# security_high — warn loud
$ loft my_script.loft
warning: web 0.1.0 has a known security issue
  advisory: GHSA-aaaa-bbbb-cccc
  summary:  Memory disclosure in HTTP parser
  fix:      web >=0.1.2 (run `loft install web@0.1.2`)
hello world

# bug (yanked but non-security)
$ loft my_script.loft
warning: gridmesh 0.1.0 was yanked (bug)
  fix: gridmesh >=0.1.1
hello world
```

**Verification timing — when hashes get checked.**

Five orthogonal moments.  The design goal: **steady-state
script runs pay nothing for verification.**  Loft is being
optimised for "many runs of small scripts" (cold-start work in
CS.C1/C2/C3); per-invocation hashing would noise the wrong
axis.  Verification binds to compile-cache invalidation
instead.

| # | Moment | What's verified | Default | Off-switch |
|---|---|---|---|---|
| 1 | **Install / auto-install** | sha256 (matches `index.json`) + Ed25519 sig on tarball | always | — (cannot disable) |
| 2′ | **At compile (cache miss)** | every library in the dep graph being compiled: cached install's sha256 matches `index.json`.  Amortised into compile time — when bytecode is being regenerated anyway, hashing N libraries adds milliseconds to an operation already costing hundreds. | on | `LOFT_NO_BUILD_VERIFY=1` |
| 3 | **Advisory feed refresh** | every 24h: re-fetch `advisories.json`, verify sig | on | `LOFT_OFFLINE=1` (uses cache) |
| 4 | **Per-invocation advisory check** | each loaded (name, version) tuple compared against cached advisories — µs in-memory lookup once feed is loaded | on | none — advisories always checked when feed cached |
| 5 | **`loft audit`** | exhaustive: re-hash every cached package + advisory match for every entry in cache and current lockfile.  Ignores all caches/markers — the explicit deep-scan path. | manual | — |

**Steady-state cost** (warm bytecode cache, no source / lockfile
changes): only moment 4.  µs per run.  Effectively free.

**What triggers a recompile (and thus moment 2′ firing):**

- Source mtime drift on any `.loft` file in the dep graph.
- Lockfile changed (auto-install fired or `loft update` ran).
- Compiler version changed (different `loft --version`).
- Target changed (`--interpret` ↔ `--native` ↔ `--html`).
- **NEW** — cached install's mtime drifted since last compile.
  Catches post-install tamper of `~/.loft/registry/<pkg>-<ver>/`.
  Cost: one `stat` per loaded library on compile-cache-hit
  path (microseconds).

That last invalidation rule is the closes-the-gap addition.
Without it, a modify-cached-library-and-restore-its-mtime attack
sails through cache hits forever; with it, ANY mtime change on
the cached install triggers a recompile and the recompile path
re-hashes (moment 2′) → mismatch caught.

**Threat model honesty.**  The retired moment "per-invocation
library hash" was only catching a narrow attack model
(modify-cached-library-WITHOUT-touching-mtime) that already
assumes the attacker has write access to `~/.loft/registry/`.
At that access level, they could equally replace the loft
binary, modify shell rc to set `LOFT_NO_BUILD_VERIFY=1`, or
tamper `~/.loft/installed.toml`.  Per-invocation hashing was
paying ~5-30ms per run for ~1% of the threat surface; that
tradeoff is wrong for a "many small runs" target.
`loft audit` (moment 5) remains the explicit escape hatch for
users who want to re-hash everything on demand.

Stdlib coverage is implicit: `default/*.loft` lives inside the
loft binary via `include_str!`, so verifying the loft binary's
bytes (lib-plan 30 § Phase 30.4 — stat-on-startup with
hash-on-drift) verifies the embedded stdlib by transitivity.
No separate stdlib-file hash check needed.

**Implementation outline (~1-2 work-days):**

1. **Registry schema bump.**  `tools/validate.py` in
   `loft-lang/registry` accepts the new typed `status` field;
   keep the old free-form string accepted in input but normalise
   on emit.  Add `advisories.json` + `advisories.json.sig` to
   the gate-1 schema lint.
2. **Advisory feed maintenance.**  Document the workflow in
   `REGISTRY_SUBMIT.md`: when yanking a version for security,
   author submits a PR adding both the per-version `status` AND
   an `advisories.json` entry referencing the GHSA.  CI verifies
   the cross-reference.
3. **Loft binary — advisory loader.**  New
   `src/registry_advisories.rs`: load + verify signature +
   cache `advisories.json` with 24h TTL (verification moment 3).
   Honours `LOFT_OFFLINE=1` (use cache; error if cache empty).
4. **Loft binary — compile-time library hash** (verification
   moment 2′).  Hook in the bytecode-cache miss path: when the
   compiler is about to regenerate bytecode for a script + its
   dep graph, hash each cached library's on-disk bytes and
   compare to the entry in `index.json`.  Mismatch → refuse to
   compile.  Cache-hit path: skip hashing entirely; the cached
   bytecode already encoded the verified state.  ALSO add a
   new cache-invalidation rule: cache-hit becomes a cache-miss
   if any loaded library's on-disk mtime drifted since the
   cache was written.  Off-switch: `LOFT_NO_BUILD_VERIFY=1`
   (intended for fully-offline development against
   known-trusted local builds).  The bytecode cache key
   itself encodes the verification state, so a successful
   verify is implicitly cached alongside the compile output.
5. **Loft binary — per-invocation advisory check**
   (verification moment 4).  After a (name, version) tuple
   lands, classify against the cached advisories.  Defer
   fail/warn emission until `main`'s pre-execute point so we
   never warn for the same package twice in one run.
6. **Override mechanism.**  `LOFT_SECURITY_OVERRIDE=<id>` env
   var allows running with a `security_critical` yanked
   version, but emits a stderr audit line: `[security] override
   applied: GHSA-xxxx-yyyy-zzzz (gridmesh 0.1.1)`.  Used for
   incident response: if the user is the one INVESTIGATING the
   CVE, they need to run the vulnerable version locally.
7. **`loft audit` command** (verification moment 5).  Explicit
   exhaustive query — scans the current lockfile (project mode)
   or the global cache (script mode), re-hashes every package
   against `index.json` (ignoring `.sha256.verified` markers),
   re-checks every tuple against advisories, and reports every
   discrepancy + every affected version without running
   anything.  Exit code reflects worst severity found.

**Tests** (in `tests/registry_advisories.rs`):

- Advisory matches version → fail/warn per severity.
- Advisory doesn't match → silent.
- Cached advisories.json absent + offline → fall through with
  diagnostic warning (don't refuse, but tell user "advisory
  feed unavailable; could not check security status").
- Cached advisories.json present + offline → use cache.
- Tampered signature → refuse to use the feed; surface the
  error.
- Override env var → run + audit-log to stderr.
- `loft audit` against a fixture lockfile with multiple severities
  → exit code matches worst.

Plus, for verification timing (moments 2′ + 5 above):

- **Cold-cache compile** (bytecode cache miss): hashes fire,
  match → compile proceeds; mismatch → compile refused with
  expected vs actual sha256.
- **Warm-cache run** (bytecode cache hit, no mtime drift):
  no hashing, no I/O beyond `stat`; steady-state cost is µs.
- **Mtime drift invalidation**: modify a cached library's
  bytes + touch the file → next run sees mtime drift,
  invalidates bytecode cache, re-hashes (moment 2′) → mismatch
  caught.
- **Mtime drift WITHOUT content change** (e.g. `touch` after
  a benign rebuild): cache invalidates, re-hash succeeds,
  compile proceeds normally — slightly slower but correct.
- **`LOFT_NO_BUILD_VERIFY=1`** → moment 2′ skipped at compile
  time; explicit stderr note `[verify] build verify disabled
  (LOFT_NO_BUILD_VERIFY)`.  Used for offline dev against
  locally-trusted libraries.
- **`loft audit` mismatch detection**: tamper a cached
  install (with OR without mtime restoration) → audit reports
  the bad sha256; exit code reflects worst severity.  This
  catches the `modify-then-restore-mtime` attack that bypasses
  moment 2′'s mtime trigger.

**Open questions:**

1. **Override audit storage.**  Just stderr, or also a file
   (`~/.loft/security_overrides.log`)?  Recommendation: stderr
   only — file logging adds a maintenance burden and we already
   write to stderr; users / CI who care can capture it.
2. **Range syntax for `affected`.**  Cargo's semver, Python's
   PEP 440, or a simple form?  Recommendation: pin to the
   existing loft.toml range syntax (`>=X, <Y`) for consistency.
3. **Multi-advisory aggregation.**  If 3 advisories hit one
   package, do we print all 3 or only the most-severe?
   Recommendation: print all; one line each.  Users investigating
   want the full picture.
4. **Retention beyond 90 days.**  Should advisories for
   long-ago-yanked versions stay in the feed indefinitely, or
   move to a separate "archive" file?  Recommendation: 90-day
   active + archive in `advisories-archive.json` for queries
   targeting old versions.

**Why this isn't deferred to a future plan.**  PLAN12 is the
*adoption* arc; security advisory is the trust signal that
makes wider adoption defensible.  Without 6.7, every published
loft library is one CVE away from a manual disclosure
campaign.  WITH 6.7, the registry recall mechanism is
mechanical and audit-friendly.

**`"package": "loft"` is a valid advisory entry.**  The same
schema covers the loft binary itself: a CVE in the parser, in
native codegen, in the runtime, or in any `default/*.loft`
stdlib file that didn't drain (per Phase 3.6) becomes an
advisory entry with `"package": "loft"`, version range, and
fix-in version.  The classifier in step 4 uses the SAME logic
for binary and library tuples — only the lookup key changes.
This is what permanently covers the non-drainable stdlib
floor (operators, base types, control flow, format strings,
core collection ops, bootstrap I/O) — the parts of the stdlib
3.6 deliberately leaves embedded.  Practical example:

```
$ loft my_script.loft
error: loft 0.8.4 was yanked for a security vulnerability
  advisory: GHSA-zzzz-yyyy-xxxx
  summary:  Format string evaluator allows arbitrary read in user-controlled payloads
  fix:      loft >=0.8.5 (run `loft self-update`)
```

The `loft self-update` referenced in the fix line is shipped
by [`lib_plans/30-loft-distribution/`](../30-loft-distribution/README.md)
— Phase 6.7 produces the advisory, Phase 30 provides the
mechanical fix path.  Both halves are required to make the
trust chain useful for the binary; 6.7 alone surfaces the
problem, 30 alone has no signal to act on.

