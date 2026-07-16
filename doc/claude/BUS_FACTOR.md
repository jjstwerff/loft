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

## The evidence — and how to check it yourself

You do not have to take this on trust. Inspect it:

- **About 80% of commits are agent-driven.** Of roughly 772 commits, over 620 carry a
  `Co-Authored-By: Claude` trailer — the code work was done in agent sessions, with the
  human steering. The owner lands the commits, but the trailer records who did the
  building. Check for yourself:
  `git log --grep="Co-Authored-By: Claude" -i --oneline | wc -l` against
  `git rev-list --count HEAD`.
- **The documentation is larger than many projects' whole codebase.** `doc/claude/`
  alone is about 73,000 lines. Check: `find doc/claude -name '*.md' | xargs wc -l | tail -1`.
- **The hardest parts were rewritten agent-led** — the store-lifetime / ownership
  model, the null and value model, the compatibility contract — each carried out across
  long sessions, with the human giving direction, not code. The closure records live
  under `doc/claude/plans/`.

## The claim, made concrete — the recipe

Here is the operational test. Anyone can run it:

1. Fire up any capable coding agent (Claude Code, or an equal).
2. Point it at the public repository and let it download the sources.
3. Let it read `CLAUDE.md` and load the skills in `.claude/skills/`.
4. Give it a real task — a bug, a feature, a refactor.

It will be able to do the work, because the *how* is in the repo. The matrix-first
method finds the real cause instead of the first plausible one. The codegen discipline
keeps the interpreter and the native backend honest with each other. `make ci` is the
gate. The docs explain every load-bearing invariant it must respect.

**So the continuation of loft does not depend on any one person.** It depends on the
public repository and on the existence of a coding agent — and both are available to
everyone.

## The same is true of everything around loft

The argument above is about developing the compiler. But the same foundation — a public
body of worked examples plus a written-down method — makes the *whole loop* around loft
low-friction, for the same reason: the hard cases have already been done in the open, so
the next one is rarely from scratch.

- **Debugging is fast, not deep.** You do not have to understand the whole compiler to
  fix a bug in it. You ask a capable agent, and it sees the tools the repo already
  gives it: `loft introspect` to read the IR, the bytecode, and the generated Rust; the
  `tests/dumps/*.txt` traces; the `LOFT_LOG` presets; the `loft-debug` skill's
  operational recipes; the matrix-first method that finds the real cause instead of the
  first guess. The agent picks the right instrument and fixes it quickly, because the
  instrument and the method are both right there.
- **Writing a library is mostly finding one that already fits.** The catalogue
  ([LIBRARIES.md](LIBRARIES.md)) and the `loft-libs-*` repositories already hold a wide
  public set — graphics, audio, net, game, assets, and more. A new library usually
  starts by finding the closest existing one and adapting it; the `loft-ship` skill and
  [LIBRARY_AUTHORING.md](LIBRARY_AUTHORING.md) carry it the rest of the way, to a build
  that behaves the same on all four targets. Little of it is invented from nothing.
- **Writing a program is mostly pointing at an example.** The games (moros, dryopea) and
  the browser gallery are all public — and all were written by an agent and steered by
  the owner, which makes every one of them a worked example. A new program starts by
  pointing at the one that looks closest and changing it. The playground turns that into
  a single step: type a few lines, press run, see output.

So it is not only the compiler that no single person has to hold in their head. Using
loft and fixing loft rest on the same thing: a public set of worked examples and a
documented method that any coding agent can pick up.

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

- [GOALS.md](GOALS.md) — Goal B (legible on contact) and Goal F (serve the reader): the
  goals this development model serves.
- [../../CLAUDE.md](../../CLAUDE.md) — the agent on-ramp: conventions, the documentation
  index, the disciplines.
- `.claude/skills/` — the executable disciplines an agent loads to work on loft.
- [DEVELOPMENT.md](DEVELOPMENT.md) — the workflow · [LAVITION.md](LAVITION.md) — the history.
