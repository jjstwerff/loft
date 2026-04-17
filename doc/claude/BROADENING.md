# Broadening — loft beyond games

Strategic evaluation of where loft can be useful beyond its current
game-centric framing, and what it takes to get there.

Current milestone framing (ROADMAP.md) is "Awesome Brick Buster — a
game worth sharing".  That is a good flagship demo but should not be
the language's identity.  This document records the broader-reach
analysis so the ecosystem work below can be sequenced deliberately
instead of reactively.

---

## Loft's genuine differentiators

Three capabilities that give loft a defensible identity outside games:

1. **Store-based heap as a language-level database.**  Data-heavy
   apps (servers, CRUD tools, ETL) feel coherent in a way
   Python/Ruby/Go cannot match because persistence and in-memory
   state share one model.  See DATABASE.md.
2. **WASM single-file HTML export** (`loft --html`, HTML_EXPORT.md).
   A one-command path from `.loft` source to a shareable interactive
   artifact.  Frictionless deployment of demos, tools, toys.
3. **`par` / `par_light` + store isolation** (THREADING.md).
   Approachable parallelism without shared-mutable footguns.

Everything below flows from these.  Broadening loft is mostly about
ecosystem work around them, not language rework.

---

## Domain fit matrix

| Domain | Natural fit | Gap to close |
|---|---|---|
| **CLI scripting / tooling** | Strong — readable syntax, static types, good error locality | Fast startup (CS.C1/C2/C3 const store + stdlib `.loftc`); stdlib for regex, shell, env, path, glob; single-binary installer |
| **Server-side web** | Very strong — store maps naturally to request/session/DB model; JSON landed | `server` library (WEB_SERVER_LIB.md), async / non-blocking I/O, route helpers, migrations story |
| **Embedded-DB DSL** | Unique — nothing else has store + language co-designed | Mostly packaging and "SQLite + scripting as one thing" narrative; the tech exists |
| **Data / ETL** | Good — iterators, parallel-for, DbRef, JSON | CSV/Parquet, decimal/BigInt, date/time, streaming file ops |
| **Educational language** | Good — Python-like surface, strong types, good diagnostics | Playground (Web IDE planned), tutorial content |
| **Scientific / analytics** | Weak — uphill vs Python ecosystem | DataFrame, BLAS, plotting; pursue only if a killer differentiator emerges |
| **Embedded MCU** | Not realistic near-term | Out of scope; C54.F keeps 32-bit SBCs viable as a floor, not a target |

Games remain a flagship demo (onboarding, Web IDE, visual appeal) but
become one entry point among several rather than the identity.

---

## What needs to happen

### Tier 1 — adoption blockers

Nothing broadens without these.  They are prerequisites for every
other domain.

- **1.0 stability contract** (ROADMAP.md).  No one builds production
  code in a 0.x language that can still reshape syntax.  Already on
  the roadmap; dominates the critical path.
- **Package registry + lock file** (PKG.7 + REG.1–4, PLANNING.md).
  Without `loft install <name>`, there is no ecosystem.
- **LSP + editor integrations** (SH.1 TextMate, SH.2 VS Code).  A
  language without syntax highlighting and go-to-definition feels
  unfinished regardless of merit.
- **User-facing documentation + tutorial.**  Current `doc/claude/*`
  is Claude-internal (excellent for me, opaque to a newcomer).
  Needs a "learn loft in 30 minutes" plus a cookbook.  Existing
  `doc/*.html` (gendoc) is the starting point.

### Tier 2 — domain-unlocking

Each item opens a specific segment.

- **Fast cold-start** (CS.C1/C2/C3 const store + stdlib `.loftc` +
  lazy-stdlib loading, LAZY_STDLIB.md).  Unlocks CLI scripting.
  Today's startup cost rules out shell-integration use.  See
  CONST_STORE.md, BYTECODE_CACHE.md, LAZY_STDLIB.md.
- **Standard-library breadth:**
  - *Scripting:* regex, date/time, path/glob, subprocess, env,
    logging (✓ LOGGER.md).
  - *Server:* HTTP client + server (WEB_SERVER_LIB.md), routing,
    TLS/ACME, sessions, CSRF.
  - *Data:* CSV, Parquet, decimal, streaming I/O, compression,
    crypto (hash/HMAC/AEAD).
- **Async / non-blocking I/O.**  Servers without it hit a ceiling
  fast.  `par_light` covers CPU work; network I/O needs its own
  story.  C56 `?? return` + I13 iterator protocol
  (SERVER_FEATURES.md) are partial enablers, not a complete model.
- **Coroutines / `yield`** (COROUTINE.md, planned 1.1+).  Required
  for streaming parsers, generator-based iterators, some server
  patterns.
- **FFI maturity** (FFI.1–4, PLANNING.md).  Opens access to C
  libraries — inevitable for crypto, compression, DB drivers,
  system APIs.

### Tier 3 — narrative and positioning

How people find and remember loft.

- **A "store + language" killer demo.**  A 100-line persistent
  multi-user chat or CRUD app where the store *is* the database.
  This is the pitch nothing else can make.
- **A scripting demo.**  20-line file-processing one-liner that
  rivals Python for readability and ships as a single binary.
- **A server demo.**  REST API + store-backed persistence + JSON in
  under 50 lines.
- **Brand separate from "game language."**  Brick Buster is a good
  demo of a language, not of loft's identity.  The identity should
  be **coherent data + code**, with games as one application.

---

## Pragmatic sequencing

Maximum broadening per unit effort:

1. **Finish 1.0 stability gate** — already the plan (ROADMAP.md).
2. **LSP + VS Code extension + tutorial** — ecosystem baseline;
   unblocks every other domain.
3. **Package registry + lock file** — ecosystem multiplier.
4. **Fast cold-start** (const store + stdlib `.loftc`) — unlocks
   CLI.  Cheapest broadening per line of code changed.
5. **`server` library v1 + async I/O** — unlocks web.  Largest
   addressable audience.
6. **Store-as-DB demo + narrative shift** — positioning work.
   Costs almost nothing in code, re-frames the whole project.
7. **Coroutines, FFI breadth, richer numerics** — second-wave;
   pursue as concrete demand appears.

---

## What not to do

- **Don't compete with Python in scientific computing head-on.**
  The ecosystem race is unwinnable from here; pursue that domain
  only if an exceptional differentiator emerges.
- **Don't broaden the language surface before 1.0.**  Every domain
  above is served by ecosystem and stdlib work, not by new syntax
  or new type-system features.
- **Don't chase embedded MCUs.**  The store-based model is the
  wrong shape for bare-metal.  C54.F keeps 32-bit SBCs viable as a
  floor; that is the limit of downward reach.

---

## Summary

Loft is closer to broadly useful than the current game-centric
framing suggests.  The unlock is:

**1.0 + ecosystem + one server demo + one CLI demo** — not new
language features.

Related documents:
- [ROADMAP.md](ROADMAP.md) — milestone ordering
- [PLANNING.md](PLANNING.md) — priority backlog (registry, FFI, LSP)
- [WEB_SERVER_LIB.md](WEB_SERVER_LIB.md) — server library design
- [CONST_STORE.md](CONST_STORE.md) — startup-speed prerequisite
- [PACKAGES.md](PACKAGES.md) — package format + registry
- [SERVER_FEATURES.md](SERVER_FEATURES.md) — language features for server ergonomics
