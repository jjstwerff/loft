<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Plan 54 — Stdlib fast-start (precompiled-stdlib cache)

## Status

**CLOSED — delivered by [@PLN11](../11-data-as-store/README.md) (arc D / D2b /
E) 2026-07-09.**  The precompiled-stdlib cache this plan sketched was built as
part of @PLN11's store-backed IR work, not as a standalone plan:

- **A** (cache artifact + (de)serialize parsed `Data` + DB schema) — done:
  `src/ir_store.rs::save_bundle(data, schema, path)` /
  `src/ir_read.rs::open_bundle_into(path, db) -> Data`, wired in
  `src/startup_cache.rs` (`warm_load_stdlib` / `save_stdlib_cache`); ~12× faster
  warm load.
- **B** (hash-keyed invalidation) — done: `src/cache.rs::stdlib_cache_key` keys
  on stdlib content + loft version + build id + target + feature set; rotates
  the cache path on any stdlib/toolchain change.
- **C** (CLI integration) — shipped as an **opt-in** env var
  (`LOFT_STDLIB_CACHE`) + XDG cache path, rather than default-on with a
  `--no-stdlib-cache` opt-out.  Promoting it to default-on is a small separable
  decision (invalidation-risk caution), not a reason to keep this plan open — a
  one-line `loft-lang/features` item if ever wanted.
- **D** (Miri-safe serde-not-mmap variant) — **moot**: the shipped cache is
  deliberately mmap-based, and this plan's own Miri motive was solved more
  cheaply by `cached_default()` (load-once-per-process), so D's reason to exist
  evaporated.

Tests: `tests/d2b_stdlib_cache.rs` (stdlib cache), `tests/arc_e_program_cache.rs`
(whole-program cache).  The original design sketch is retained below as
historical record.

---

### Original design sketch (historical — 2026-05-29)

**Origin:** surfaced during
[@PLAN53](../finished/53-sanitizer-ci-lever/README.md) Stage A1 sanitizer
probing (2026-05-29).  Every loft invocation re-parses the entire
`default/*.loft` stdlib at startup — lexing, two-pass parsing, scope analysis,
and bytecode generation over the whole corpus — before running a single line of
user code.  Natively this is milliseconds; it became visible because under Miri
(10–100× interpretation overhead) the same load takes **minutes**, which is
what makes a Miri CI subset look unusable.

This plan ships a **precompiled-stdlib cache**: serialize the
parsed stdlib state once, key it on a content hash of
`default/*.loft` (+ loft version), and on startup deserialize the
cached artifact instead of re-parsing when the hash matches.
Think Python `.pyc` / a compiler's prebuilt prelude.

It is **off PLAN53's critical path** — PLAN53's Miri affordability
is solved more cheaply by `cached_default()` (load-once-per-
process) + minimal fixtures.  This plan is the broader,
user-facing startup-latency win, filed so the idea (and its
Miri-provenance caveat) isn't lost.

## Goal

On startup, load the parsed stdlib from a hash-validated on-disk
cache in O(deserialize) instead of O(parse), transparently
rebuilding the cache when `default/*.loft` or the loft version
changes — with no change to observable program behaviour.

## Effort + design

- **Effort:** M–MH
- **Design:** ~ (partial — serialization mechanism is the open question)
- **Last touched:** 2026-05-29

## Sub-arcs

| Item | Source | Status |
|---|---|---|
| **A** — Cache artifact + (de)serialization of the parsed `Data` / `Database` (incl. `DbRef` fixup, store heap) | this README § Open design questions Q1 | Open |
| **B** — Hash-keyed invalidation: content hash of `default/*.loft` + loft version + feature flags → cache key; rebuild-on-mismatch | this README | Open |
| **C** — CLI integration: cache location (`~/.cache/loft/` vs `target/`), `--no-stdlib-cache` opt-out, first-run build + atomic write | this README | Open |
| **D** — Miri-safe build variant: serde-into-fresh-allocations, **not** mmap raw-byte reinterpret (see Q2) | this README § Open design questions Q2 | Open |

## Phase ordering

1. **A** first — the serialization format is the load-bearing
   decision; everything else is plumbing.  Prototype against the
   in-memory `Data` clone path (`cached_default()` already proves
   the state is cloneable; serialization is the durable form).
2. **B** — once a blob round-trips, add the hash key + rebuild
   trigger.  Cheap and well-bounded.
3. **C** — wire into `src/main.rs` startup, behind an opt-out.
   Atomic write (temp + rename) so concurrent invocations don't
   tear the cache.
4. **D** — only if PLAN53 elects to run a Miri CI subset against a
   cached stdlib.  Gate the deserialize path to the
   provenance-safe variant under `cfg(miri)` or unconditionally.

## Open design questions

1. **Serialization mechanism.**  Two candidates:
   - **Reuse [@PLN43 loft-store-durable](../43-loft-store-durable/README.md)** mmap/durable-store machinery — the store is already a word-addressed heap with a durable-format arc in flight.  Cheapest if PLAN38 lands a serializable store.  **But:** see Q2 — the mmap-reinterpret form is Miri-hostile.
   - **Plain `serde` into freshly-allocated Rust structures** — slower to materialize than mmap, but provenance-clean and Miri-safe.
   The `DbRef` fixup (pointers are `(store_nr, rec, pos)` — already position-independent, see [DATABASE.md](../../DATABASE.md)) may make this cheaper than a typical pointer-graph serialize, since `DbRef` is not a raw machine pointer.

2. **Miri provenance caveat (the reason this was filed).**  An
   mmap'd cache that `transmute`s file bytes into the typed store
   is **UB under Miri** — Miri cannot track pointer provenance
   through bytes that originate outside the Rust abstract machine,
   so it would reject or mis-track the reinterpreted store.  A
   plain-serde variant that allocates fresh `Vec`/`String`/store
   words and copies in is Miri-clean.  If PLAN53 ever wants Miri
   to run against a cached stdlib, arc D MUST take the serde form,
   not the mmap form.

3. **Cache key composition.**  Content hash of every
   `default/*.loft` file + the loft semantic version + the active
   feature-flag set (the stdlib bytecode differs by feature, e.g.
   `random` / `threading`).  Miss → rebuild + rewrite.

4. **Where the cache lives.**  `~/.cache/loft/stdlib-<hash>.bin`
   (XDG, survives across checkouts) vs `target/` (per-build,
   simpler, ignored by git).  Library/IDE embedders may want an
   explicit path override.

5. **Correctness gate.**  A cache hit must be byte-for-byte
   equivalent to a fresh parse.  Need a CI check that runs the
   suite once cold (cache miss) and once warm (cache hit) and
   diffs nothing — the cache must never change observable
   behaviour, only latency.

## Cross-arc dependencies

- **[@PLN43 loft-store-durable](../43-loft-store-durable/README.md)** —
  shares the store-serialization problem.  If PLAN38 ships a
  durable/serializable store, arc A can build on it (with the Q2
  caveat for the Miri variant).  Sequence PLAN54 after PLAN38's
  serialization primitive when possible.
- **[@PLAN53 sanitizer-ci-lever](../finished/53-sanitizer-ci-lever/README.md)** —
  this plan is the generalization of PLAN53's "Miri-subset tests
  must not reload the full stdlib per test" finding.  PLAN53 does
  NOT depend on PLAN54 (it uses `cached_default()` + minimal
  fixtures instead); PLAN54 carries the Miri-provenance constraint
  PLAN53 surfaced.

## See also

- [DATABASE.md](../../DATABASE.md) — store allocator, `DbRef`
  layout (position-independent → serialization-friendly).
- [COMPILER.md](../../COMPILER.md) — the parse → IR → bytecode
  pipeline whose output this plan caches.
- [PERFORMANCE.md](../../PERFORMANCE.md) — startup-latency context;
  this plan's win is amortizing the per-invocation stdlib parse.
- [@PLN43 loft-store-durable](../43-loft-store-durable/README.md),
  [@PLAN53 sanitizer-ci-lever](../finished/53-sanitizer-ci-lever/README.md).
