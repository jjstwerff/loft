<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# Bus factor — loft doesn't have one

> **The short version.** Everything needed to develop loft is in this repository, and
> it is public. Point any capable coding agent at it, let it load the skills in
> `.claude/skills/`, and it can make the changes loft needs — investigate the problem,
> fix it at the right place, verify on both backends, and land it through `make ci` —
> with no single person holding the project in their head. The knowledge lives in the
> repo, not in a founder.

## The worry, stated fairly

Look at loft from the outside and one conclusion seems obvious. It is a solo-built,
ambitious systems project: a statically-typed language with a tree-walking interpreter
**and** a native (`rustc`) backend, a store-based heap, an ownership model, a
cross-target package system. Deep, interlocking parts; one person steering. So the
**bus factor** must be high — if that one person stops, the project dies. For almost
every solo project, that reasoning is correct.

## Why it is wrong here

What makes a solo project fragile is not that one person writes it. It is that the
**knowledge of how to work on it lives in that person's head**, undocumented, so nobody
else can pick it up. loft was built to remove exactly that. The how-to is written down,
in the public repo, in a form a machine can act on:

- **The full source is public** — the interpreter, the native code generator, and the
  standard library. Nothing is withheld.
- **About 73,000 lines of documentation** (`doc/claude/`, 600+ files): the
  architecture, the design decisions and *why* each was made, the debugging methods,
  the memory model, the edge cases, and a plan record for every major piece of work.
  That is roughly one line of documentation for every three lines of source.
- **Ten skills** (`.claude/skills/`) — not notes, but **executable disciplines** an
  agent loads and follows. How to fix a bug without guessing (`engineering-rigor`:
  build the boundary matrix, fix at the one chokepoint). How to change code generation
  safely (`loft-codegen`: prove the working bytecode on both backends *before* editing
  the compiler). How to run a crash down (`loft-debug`). How to commit to a load-bearing
  design (`design-protocol`). How to write and test `.loft`; how to ship a library so it
  behaves the same on all four targets. These encode the *method* — the part that
  normally lives only in a senior maintainer's instincts.
- **One-command tooling.** `make ci` runs the whole gate (format, lint, the full test
  suite). `find_problems.sh` runs the suite in the background and hands back every
  failure. `loft introspect` dumps the IR, the bytecode, and the generated Rust, so an
  agent can *see* what the compiler does. The tracker (`./scripts/idx`) resolves every
  plan and issue offline.

Put together: an agent that clones this repo has the source, the reasons, the method,
and the instruments. That is the whole job.

## The design choice behind it

This did not happen by accident. The owner **prioritized documentation and tooling
above writing code himself**, on purpose. His job was to **steer** — to set direction
and make the calls a human should make: what to build next, which one-way doors are
worth walking through, when a design is good enough. The building — including large,
careful rewrites that would normally need close hand-holding — was done by a coding
agent following the disciplines in the repo. Every lesson learned was written back into
a doc or a skill (a standing rule of the project: *durable knowledge must land in the
repo, not stay in one agent's private memory*), so the next agent starts where the last
one finished.

That is loft's development model, stated plainly: **a human with taste, a coding agent
with capability, and a repository that carries the method between them.** None of the
three is a specific, irreplaceable person.

## Everything on this page is independently verifiable

Take none of this on trust. Every claim here is checkable **without a word from anyone**
— clone the repo and run the command, or open the public link. That is the point: the
argument stands on evidence you can reproduce, not on assertion.

| Claim | Check it yourself |
|---|---|
| The full source is public — nothing withheld | Clone <https://github.com/loft-lang/loft> |
| ≈80% of commits are agent-driven, human-steered | In the clone: `git log -i --grep="Co-Authored-By: Claude" --oneline \| wc -l` (≈620) against `git rev-list --count HEAD` (≈772) |
| ≈73,000 lines of documentation | `find doc/claude -name '*.md' \| xargs wc -l \| tail -1` |
| Ten executable skills carry the *method* | `ls .claude/skills/` — then read any `SKILL.md` |
| One command runs the whole gate | `make ci` (format, lint, full test suite) |
| The compiler is fully inspectable | `loft introspect any.loft` — dumps the IR, bytecode, and generated Rust |
| The tests are real and pass in **public CI** | <https://github.com/loft-lang/loft/actions/workflows/ci.yml> |
| The hardest parts were rewritten agent-led | The closure records under `doc/claude/plans/`; the commit history + its co-author trailers |
| A public library catalogue exists | `doc/claude/LIBRARIES.md`; the `loft-libs-*` repos at <https://github.com/loft-lang>; `loft search <keyword>` |
| Programs run live in the browser | Playground <https://loft-lang.org/loft/playground.html> · Gallery <https://loft-lang.org/loft/gallery.html> |
| **Many public example programs, varied domains** | <https://github.com/jjstwerff> — `crawler` (a hex roguelike in loft), `dryopea` + `moros` (games), `routing` (a phone-first route planner), `ssh_home` (a pure-loft phone SSH terminal), `zero-trust-shared-files` (a federated file system) |
| The language + stdlib are documented | <https://loft-lang.org/loft/> |
| The issue tracker is public | <https://github.com/loft-lang/loft/issues> |

## The recipes — exactly how anyone does each thing

The operational test, made specific. Each recipe assumes what the model assumes: a
person, plus a coding agent pointed at the repo (it reads `CLAUDE.md` on start and can
load the skills in `.claude/skills/`). None of these needs a maintainer's help.

### Fix a bug in loft (the compiler or runtime)

1. `git clone https://github.com/loft-lang/loft && cd loft && cargo build`.
2. Start a coding agent in the folder. Have it load the `engineering-rigor` skill (the
   matrix-first method); add `loft-codegen` for compiler/codegen work, or `loft-debug`
   for a crash.
3. Reproduce with the smallest `.loft` you can, on **both** backends:
   `cargo run --bin loft -- bug.loft` and `cargo run --bin loft -- --native bug.loft`.
4. See what the compiler actually does:
   `cargo run --bin loft -- introspect bug.loft` (IR + bytecode + generated Rust); read
   the matching `tests/dumps/*.txt`; narrow with `LOFT_LOG=crash_tail:50`.
5. Build the boundary matrix (the skill), find the one chokepoint, and fix it there —
   no wider.
6. Add a regression test (`tests/scripts/NNN.loft` or `tests/*.rs`); verify on both backends.
7. Run the gate: `make ci`, or `./scripts/find_problems.sh --bg` then `--wait` for the
   full suite in the background.
8. Commit on a feature branch and push.

### Write a library

1. Find the closest one that already exists: browse `doc/claude/LIBRARIES.md` and the
   `loft-libs-*` repos at <https://github.com/loft-lang>, or run `loft search <keyword>`.
2. Scaffold a fresh one: `loft new <name>` (writes `loft.toml` + `src/`).
3. Write the `.loft`, adapting the nearest example. Have the agent load the `loft-write`
   skill for syntax; the reference is `doc/claude/LOFT.md` + `STDLIB.md`.
4. Make it work on every target: load the `loft-ship` skill and confirm identical
   behaviour on the interpreter, `--native`, `--native-wasm`, and `--html`.
5. Test it, then publish: `loft publish` (a touch-gated signature) — or keep it local
   with `loft install <dir>`.

### Start a program

1. Point at the nearest public example — there are many, across very different domains,
   at <https://github.com/jjstwerff>: `crawler` (a hex roguelike), `dryopea` and `moros`
   (games), `routing` (a route planner), `ssh_home` (a phone SSH terminal),
   `zero-trust-shared-files` (a federated file system), plus the browser Gallery
   (<https://loft-lang.org/loft/gallery.html>). The fastest first look is the Playground
   (<https://loft-lang.org/loft/playground.html>): type a few lines, press run, see output.
2. Locally: copy the closest example into a `.loft` file and change it.
3. Pull in the libraries you need: `loft install <name>`, then `use` them in the file.
4. Run it: `loft myprog.loft` (or `cargo run --bin loft -- myprog.loft` inside the repo).
5. Have the agent load `loft-write` for syntax as you go; the reference is `doc/claude/LOFT.md`.

### Edit a program that already exists

1. Clone or open it — the example programs are public repos under
   <https://github.com/jjstwerff> (`crawler`, `dryopea`, `moros`, `routing`, `ssh_home`,
   `zero-trust-shared-files`, …), and the libraries under <https://github.com/loft-lang>.
2. Run it first to see the current behaviour: `loft prog.loft`.
3. Make the change. `loft-write` + `doc/claude/LOFT.md` keep the syntax right;
   `loft introspect prog.loft` shows what the compiler makes of it if something is off.
4. Re-run it and run its tests — every loft repo carries the same conventions
   (`loft --tests tests/`, `make ci`).
5. Commit and push.

**So the continuation of loft — building it, using it, or fixing it — does not depend on
any one person.** It depends on the public repository and on the existence of a coding
agent, and both are available to everyone.

## What "no bus factor" does and does not mean

Be precise. It does **not** mean no human judgment is ever needed: someone still has to
choose what loft should become, and make the value calls a machine should not make
alone. It **does** mean that role is not tied to one specific person. Anyone with taste
and a coding agent can pick it up, because the agent supplies the systems capability and
the repo supplies the method. The fragile thing — deep, undocumented systems knowledge
held in a single head — was engineered out. Lose any one contributor, human or agent,
and the repository plus a fresh agent carry on.

That is the reason for building in the open, for documenting above coding, and for
teaching the method to the tools: **loft is not one person's project that others may
read. It is a project anyone with a coding agent can continue.**

## See also

- [AGENT_ACCOUNT.md](AGENT_ACCOUNT.md) — a first-person account from the Claude agent that
  did most of loft's building, after it checked the record of the past sessions.
- [GOALS.md](GOALS.md) — Goal B (legible on contact) and Goal F (serve the reader): the
  goals this development model serves.
- [../../CLAUDE.md](../../CLAUDE.md) — the agent on-ramp: conventions, the documentation
  index, the disciplines.
- `.claude/skills/` — the executable disciplines an agent loads to work on loft.
- [DEVELOPMENT.md](DEVELOPMENT.md) — the workflow · [LAVITION.md](LAVITION.md) — the history.
