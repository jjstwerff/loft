
# Claude Code Instructions for the Loft Project

> These instructions OVERRIDE default behavior — follow them exactly. The MANDATORY
> sections (branch / debugging / bug-filing / git-safety) are hard rules.

## What loft is

**loft** is a tree-walking interpreter (Rust) for the **loft** language: statically typed,
expression-oriented, struct/enum, store-based heap, stdlib from `default/*.loft`. Two backends:
the interpreter and `--native` (compiles via `rustc`). It's the **language** layer of a stack:
**lavition** (the engine/brand) → **loft** (this repo) → games **moros**/**dryopea** + consumer
libs (crawler, `lib/markdown`) that dogfood the language. History: [LAVITION.md](doc/claude/LAVITION.md).
Developed almost entirely by AI agents (steered; docs + tooling prioritized above code), so
everything needed to work on loft is in this repo — [BUS_FACTOR.md](doc/claude/BUS_FACTOR.md).

## Dogfood loop

**Build a real consumer → harvest the lessons → fix the language → ship.** Prefer the path that
exercises a real consumer; when a slice surfaces a gap, fix on the spot if XS/S, else route to its
canonical home ([DEVELOPMENT.md § Inserting Discovered Enhancements](doc/claude/DEVELOPMENT.md#inserting-discovered-enhancements-into-the-active-plan)).
**Two-agent split:** this stream BUILDS + FIXES the language and documents the contract; the
consumer's own agent USES + adversarially BREAKS it and reports gaps.
**Edit ONLY this repo** — the symmetric half of the consumer's "the engine is read-only" rule.
Read their tree freely (source, docs, `git log`, their `LOFT_HANDOFF.md`); never write to it. They
are often working in it concurrently, so a staged test file or a `git checkout` lands in someone
else's uncommitted work. Verify a consumer-reported bug from a **scratchpad** package that points at
their libs by path (`--lib <their>/lib`, or a `path =` dep) — note `loft test` inside their package
is NOT read-only: it rebuilds `native-auto/`, writes `.loft/` caches, and a file-writing test can
delete a tracked file of theirs. Report back in our docs, not in their files.

## Key commands

```bash
cargo run --bin loft -- prog.loft        # run     |  -- repl  |  -- introspect prog.loft  |  -- --help
loft debug prog.loft:12 [--lib dir]      # STOP at line 12: read/edit the live frame, step
                                         #   (pipe commands on stdin; `--rpc` = scripted NDJSON)
                                         #   reach for this INSTEAD of adding println — DEBUG.md
cargo run --bin gendoc                   # regenerate doc/*.html
make ci                                  # fmt → clippy → test (full local gate)
make test                                # clippy + test → result.txt
./scripts/find_problems.sh --bg|--peek|--wait   # background full-suite run + inspect/block
make speed                               # what got slower/faster — a REPORT, never a gate
make profile ARGS="--interpret p.loft"   # which loft FN/LINE/PATH burns the time; PROFILE_FLAGS=
                                         #   "--mem" heap by loft line at the PEAK, "--paths" the
                                         #   paths that reached each allocation, "--engine" perf
                                         #   over loft's own Rust.  `make profile-corpus` checks
                                         #   the instruments against known answers — PERFORMANCE.md
make index ; ./scripts/idx tag:@P259     # rebuild + query the tracker index (prefer over grep -rn)
make view                                # branch-aware doc/code viewer (SSH-forward 8765)
```

**Bound ad-hoc runs** (loft is unbounded by default; tests already arm a 300s watchdog). Especially
for `--native` (rustc can hang): `LOFT_TIMEOUT=60 loft --native p.loft` or `loft --timeout 60 p.loft`
(0 = off). Hard-kills at `timeout+grace` (grace 2s, `LOFT_TIMEOUT_GRACE`). Ref: DEBUG.md, TESTING.md.

**A time bound does not bound MEMORY.** A corrupted length ends in a bad dereference on one run
and an unbounded ALLOCATION on the next — loft#796 reached 59.6 GiB in seconds and the global OOM
killer took two unrelated agent sessions with it. Test runs (`--tests` / `loft test`) therefore
carry a **2 GiB store-heap ceiling**; crossing it stops the run at that growth and names the TYPE
that filled the heap, with a one-store-vs-many breakdown that tells a runaway length from a leak.
`LOFT_MEMORY_LIMIT=<2G|512M|0>` overrides it; ordinary runs are never capped. When writing a
repeat-run harness for a corruption repro, cap the process too (`ulimit -v`) — the runaway is not
necessarily the process the kernel kills. TESTING.md § Store-memory ceiling.

**Under debug assertions a third bound applies:** the interpreter stops after `LOFT_MAX_OPS`
operations (default 4e9, `0` = off) and prints the last sixteen ops as `function+offset: OpName` —
reach for it, set LOW, when hunting a hang, because it names the loop a timeout can only time out
in. Absent from every release build. It is a count, so it cannot tell a long run from a hung one:
at 100M it was tripping legitimate library tests and reporting them as infinite loops, which read
the debug-assertions gate as known-red (loft#919). TESTING.md § Hang guard.

For any multi-failure refactor, start `find_problems.sh --bg` before editing (detached
`cargo test --release --no-fail-fast` → `/tmp/loft_problems.txt`).

## Tracker tags <!--noindex-->

`@`-prefixed so regex is unambiguous: **`@P259`** P-issues; **`@PLN3`** = a `loft-lang/plans` issue
(canonical; plan id = issue number); **`@PLAN22`** = legacy local plan dir (migrating to `@PLN`);
**`@F7`/`@I81`** = a `loft-lang/features` issue (the feature/infra catalogue, @PLN92) — the ISSUE
is canonical; `index/features.json` + `doc/features/` + `tests/docs/features/*.loft` are its
GENERATED shadow (never edit them: edit the issue, then `make features-fetch && make features-gen`;
the `features-check` drift guard fails on hand-edits).
File a NEW plan as a `loft-lang/plans` issue with one `status:*` + one `subject:*` label. Look refs
up with `./scripts/idx` (`make index` first if stale; `./scripts/idx help` for queries).

## Architecture — execution path

```
src/main.rs            CLI; loads default/ then user file
 └ src/parser/         two-pass recursive-descent → Value IR
     mod.rs(core) definitions.rs(enum/struct/typedef/fn) expressions.rs(expr/assign)
     operators.rs(op dispatch/coercion) vectors.rs fields.rs(index/field) objects.rs(vars/construct)
     collections.rs(for/map/filter/par) control.rs(match/parse_call) builtins.rs
   src/lexer.rs  src/typedef.rs(type resolution+offsets)  src/variables/  src/scopes.rs(scope/lifetime)
 └ src/compile.rs      IR → flat bytecode; inits native registry
 └ src/state/          executes bytecode: mod.rs(State/execute) text.rs io.rs(file/db) codegen.rs debug.rs
   src/fill.rs         opcode implementations
```

## Key data structures

| Type | File | Purpose |
|---|---|---|
| `Value` / `Type` / `Data` | `src/data.rs` | IR node / static type / table of named defs |
| `State` | `src/state/mod.rs` | bytecode stream + runtime stack |
| `Stores` / `Store` | `src/database/mod.rs` / `src/store.rs` | all stores+schema / raw word-addressed heap |
| `DbRef` | `src/keys.rs` | universal pointer (store_nr, rec, pos) |

## Conventions

- User fns stored as `"n_<name>"` — `data.def_nr("n_foo")`, not `"foo"`.
- Native stdlib: globals `n_<func>`; methods `t_<LEN><Type>_<method>` (LEN = chars in type name),
  e.g. `t_4text_starts_with`. Operators `OpCamelCase` (loft) → `op_snake_case` (`fill.rs`).
- `#rust "..."` in `default/*.loft` supplies the Rust body for codegen. Full naming + null-sentinel
  rules: [CODE.md](doc/claude/CODE.md).
- stdlib load order: `01_code.loft` (operators/math/text/collections) → `02_files.loft` (I/O) →
  `03_text.loft`.
- **Before non-trivial functionality, check the library catalogue (`make libcatalogue`) + `loft install`** — don't reimplement.
  Writing/reviewing `.loft`: **loft-write skill**. Language ref: [LOFT.md](doc/claude/LOFT.md), [STDLIB.md](doc/claude/STDLIB.md).
- **A library's API: build the catalogue with `make libcatalogue`, then read the (local, git-ignored)
  `doc/claude/LIBRARIES.md` — NEVER a clone or installed copy (@PLN112).** The catalogue is a **local
  build, not committed data** — a generated view of `published` + each lib's `origin/main` (breakage-
  flagged), rebuilt on demand so it can't go stale (committing it only churned the repo for no benefit);
  a local clone / `~/.loft/registry/<pkg>-<ver>/` can silently lag `origin/main`
  (the `find`→`search` failure that motivated @PLN112). For the machine-/context sources run
  the overlay: `scripts/lib-overlay.py <name>` (local checkout + this project's pin),
  `scripts/proposal-review.py <name> <ref>` (a proposed candidate). We never auto-delete a
  copy — each is a legitimate source.
- **User-facing output** (anything a command PRINTS): silence when nothing needs acting
  on; no plan tags / phase names / "not yet implemented" in it; the full explanation only
  on failure. loft is meant to be BORING — noticed only in its absence
  ([GOALS.md](doc/claude/GOALS.md), [DOC_QUALITY.md § D](doc/claude/DOC_QUALITY.md)).
- **Response shape:** lead with the ONE highest-leverage item in full (decision + minimum to act),
  then a one-line summary of the rest; no long dumps.
- **Advice vs action:** asked for advice/evaluation → give the recommendation (best option + why),
  don't bounce it back. Asked a question → answer it. Act/edit only on an explicit do-it instruction.
- **Shared knowledge:** anything durable an agent records to private memory must ALSO land in the
  right repo doc (memory is per-agent; the repo is the shared channel). Keep machine-specific values
  out of shared docs.

---

## Branch policy — MANDATORY

`main` is the release branch; every commit on it must be releasable.
1. **Never commit on `main`** — land on a feature branch, reach main only via a GitHub PR (never a
   local `git merge`).
2. **Push proactively — it's a SAFETY rule.** Once a change settles (compiles / tests green), commit
   + push to the feature branch so it isn't lost. Separate from opening a PR.
3. **Never create a branch, open, or merge a PR without an explicit user ask** ("create PR",
   "merge", "switch branch"). "fix X" / "push" / "retry" are NOT such asks; a prior ask doesn't carry
   over. If a protected branch blocks a commit, surface it and ask — don't invent a branch.
4. With an **open PR**, hold non-blocking pushes for the user's consent (force-push/rebase/surprise
   commits) — EXCEPT a push that unblocks a red required check (allowed; it can't merge while red).
5. **While a PR is unmerged, branch from the TIP of that in-flight work — NEVER fork a fresh
   branch off `main`.** `main` lacks the unmerged foundation, so a `main`-based branch can't build
   on it and **development there is impossible** (new work almost always needs what's still in the
   open PR — e.g. @PLN85's fuzz-proof needs @PLN25, which sits in the PR). Stack the new branch on
   the PR branch; rebase the whole stack onto `main` only AFTER the PR merges. Fork from `main`
   only when there is no in-flight work to build on. **Trade-off (respect it):** stacking couples
   the new work to the PR's merge clock — a clean PR merges in minutes, but a problematic one
   blocks everything stacked on it for **hours**, so keep the PR mergeable and land it promptly.
   General branch name; prefix agent branches with the hostname (`<host>-work`); the cycle's
   long-lived branch is the **monthly release branch** `YYYY-MM`; only a substantial design-doc'd
   plan earns a specific name.
6. **Before opening a PR and before requesting merge, verify the head is current on `origin/main`**
   (`git fetch`; `git merge-base --is-ancestor origin/main <head>`). `mergeStateStatus: BEHIND`
   merges as BLOCKED even when `mergeable=MERGEABLE` — rebase + re-push first.

## Debugging policy — MANDATORY

**Never `git bisect` or `git checkout HEAD -- <file>`** — both destroy uncommitted work. Instead:
read the failing test's dump (`tests/dumps/*.txt` — IR+bytecode+trace), narrow with
`LOFT_LOG=minimal`/`crash_tail:50`, read the 3–5 relevant files, `git show <commit>` for regressions.

**Matrix-first for any non-trivial bug (esp. crash / silent corruption) — the urge to fix is the
signal you haven't earned it:**
1. Don't fix on the first read — a clean one-line story is a hypothesis.
2. Build the matrix in throwaway `/tmp` probes on `--interpret` (`scripts/probe-matrix`), varying ONE
   composition axis per probe, distinctive values everywhere. But **count the axes you HELD FIXED** —
   a sweep varying one while pinning four reads as proof and isn't (@PLN130's broken cell needed
   nesting depth, Set-count, param kind and caller-count moved TOGETHER).
3. **Hand-compute each cell's expected value** (agreement between two binaries is NOT a pass);
   prove the harness can fail (a no-output cell is vacuous); assert **value AND length AND leak**.
4. Map pass/fail → find the REAL boundary (filed scope is usually wrong). Resisting a read twice →
   instrument with one env-gated `eprintln`, don't theorize.
5. Fix at the chokepoint enforcing exactly the violated invariant — no narrower, no wider.
6. Verify the full matrix on **BOTH backends**; graduate guarantee probes to `tests/scripts/`.
7. **Propose 3+ cases to check → write them ALL down BEFORE working the first.** Detail decays
   while you work: the headline of case three survives, its specifics (which axis, which shape,
   why suspected) do not. Writing the list first also makes it reviewable while it's cheap.
8. **A new case found = a new probe, always** — the suite is the only thing that remembers.

Full flow: [DEBUG.md](doc/claude/DEBUG.md), [plans/_INVESTIGATION_TEMPLATE.md](doc/claude/plans/_INVESTIGATION_TEMPLATE.md).

## Bug-filing policy — MANDATORY

**Default is FIX, not file** — bugs surfaced while fixing another are the cheapest to fix (paths
loaded, repro warm). In **stability work** the file-instead-of-fix escape hatches do NOT apply: fix
in the same session with a regression test. Record scope + root cause, never origin commit.

**File only when NOT fixing now:** it blocks the current task (bookmark + workaround), or it's
genuinely M+/needs-design (route to its canonical home). When you file: a **GitHub Issue**
(`gh issue create`, `bug_report` template) — NOT a PROBLEMS.md row (that's the closed archive) —
with a minimal both-backend repro, `sev:`/`area:` + a VERIFIED `wa:*` label, and `Fixes #NNN`.

**Fixing an existing issue not yet on `main`:** push the fix, write `Fixes #NNN`, keep the issue open
(the `fixed-pending-merge` label is automated off that trailer) — never hand-close. **Inside a
plan:** file only if it reproduces on `main`; branch-internal breakage stays in the plan's docs.
Don't scope-creep the active fix with unrelated bugs.

## Git safety — MANDATORY

**Never `git stash pop` / `git pull` / `git checkout HEAD -- <file>` with uncommitted changes** —
they merge-conflict across files and have destroyed sessions. **Always commit before any operation
that changes the working tree.** Compare without switching: `git diff main -- <file>`,
`git show origin/main:<file>`.

Run **`make hooks`** once per clone: the `commit-msg` hook reports an issue mentioned without a
`Fixes #N` trailer (that trailer is what the push workflow labels `fixed-pending-merge` off — see
the bug-filing policy above). It never blocks.

---

## Documentation index

**Language / stdlib:** [LOFT.md](doc/claude/LOFT.md) syntax · [STDLIB.md](doc/claude/STDLIB.md) stdlib API ·
[INTERFACES.md](doc/claude/INTERFACES.md) traits/generics · [TUPLES.md](doc/claude/TUPLES.md) ·
[COROUTINE.md](doc/claude/COROUTINE.md) (1.1+) · [INCONSISTENCIES.md](doc/claude/INCONSISTENCIES.md).

**Compiler / internals:** [COMPILER.md](doc/claude/COMPILER.md) parser/two-pass/types ·
[INTERMEDIATE.md](doc/claude/INTERMEDIATE.md) Value/Type/opcodes/State · [INTERNALS.md](doc/claude/INTERNALS.md) ·
[SLOTS.md](doc/claude/SLOTS.md) stack slots · [NATIVE.md](doc/claude/NATIVE.md) `--native` codegen ·
[THREADING.md](doc/claude/THREADING.md) par · [CODEGEN_METHOD.md](doc/claude/CODEGEN_METHOD.md) how to do compiler work.

**Runtime / memory:** [DATABASE.md](doc/claude/DATABASE.md) stores/DbRef ·
[REMOTE_STORES.md](doc/claude/REMOTE_STORES.md) serving static data over HTTP range (paged
`store_load_key*`, no server-side code) · [LAZY_STORES.md](doc/claude/LAZY_STORES.md) a collection
bound to an image or `sqlite:` fetches on a MISS, query derived from its own type ·
[LIFETIME.md](doc/claude/LIFETIME.md) deps/freeing ·
[OWNERSHIP_MODEL.md](doc/claude/OWNERSHIP_MODEL.md) the deps north-star (borrow system) ·
[PLACEMENT.md](doc/claude/PLACEMENT.md) a library runs in this process, a worker, or another
machine — one manifest line, consumers unchanged; **the four rules for writing one that can
be placed** (a `pub fn` must not BE a native; answer a value, not a cursor; closures do not
cross; a returned VIEW cannot be placed) ·
[LOGGER.md](doc/claude/LOGGER.md) · [WASM.md](doc/claude/WASM.md) · [HTML_EXPORT.md](doc/claude/HTML_EXPORT.md) ·
[BROWSER_INTEROP.md](doc/claude/BROWSER_INTEROP.md) · [WINDOWS.md](doc/claude/WINDOWS.md) / [WINDOWS_SESSION.md](doc/claude/WINDOWS_SESSION.md).

**Diagnostics:** [DIAGNOSTICS.md](doc/claude/DIAGNOSTICS.md) the code index (`advice[avoidable-copy]`)
+ `--explain` fix lines — a code is a FROZEN public surface, and a new one lands with its row.

**Testing / debug:** [TESTING.md](doc/claude/TESTING.md) framework/`LOFT_LOG`/LogConfig ·
[DEBUG.md](doc/claude/DEBUG.md) tools + boundary-matrix runner · [CAVEATS.md](doc/claude/CAVEATS.md) edge cases ·
[PERFORMANCE.md](doc/claude/PERFORMANCE.md) benchmarks · [CI_BUDGET.md](doc/claude/CI_BUDGET.md) what runs
when + the 20-min PR rule.

**Quality / stability / formal:** [CODE.md](doc/claude/CODE.md) · [DOC_QUALITY.md](doc/claude/DOC_QUALITY.md) ·
[QUALITY.md](doc/claude/QUALITY.md) open work · [GOALS.md](doc/claude/GOALS.md) (purpose + goals A–F) ·
[BUS_FACTOR.md](doc/claude/BUS_FACTOR.md) (the development model — repo + agent, no single point of failure) ·
[STRONG_POINTS.md](doc/claude/STRONG_POINTS.md) · [DESIGN.md](doc/claude/DESIGN.md) algorithms ·
[DESIGN_DECISIONS.md](doc/claude/DESIGN_DECISIONS.md) declined-features register ·
[DESIGN_PROTOCOL.md](doc/claude/DESIGN_PROTOCOL.md) / [DESIGN_VERIFICATION.md](doc/claude/DESIGN_VERIFICATION.md) ·
[FORMATTER.md](doc/claude/FORMATTER.md) · stability: [STABILITY_ROADMAP.md](doc/claude/STABILITY_ROADMAP.md)
(the tracking view) · [STABILITY_METHOD.md](doc/claude/STABILITY_METHOD.md) /
[_SWEEP](doc/claude/STABILITY_SWEEP.md) / [_HOTSPOTS](doc/claude/STABILITY_HOTSPOTS.md) /
[_REDFLAGS](doc/claude/STABILITY_REDFLAGS.md) · [DEPS_INVENTORY.md](doc/claude/DEPS_INVENTORY.md) ·
formal lens: [FORMALIZATION.md](doc/claude/FORMALIZATION.md) / [TYPING_RELATION.md](doc/claude/TYPING_RELATION.md) ·
strict: [formal/README.md](doc/claude/formal/README.md) (rules + deviations driven to zero).

**Plans / roadmap:** [plans/README.md](doc/claude/plans/README.md) · [PLANNING.md](doc/claude/PLANNING.md) backlog ·
[ROADMAP.md](doc/claude/ROADMAP.md) by milestone · [BROADENING.md](doc/claude/BROADENING.md) beyond games ·
[lib_plans/README.md](doc/claude/lib_plans/README.md) (legacy) · [STACKTRACE.md](doc/claude/STACKTRACE.md) · [SANDBOX.md](doc/claude/SANDBOX.md).

**Libraries / registry / packages:** `LIBRARIES.md` (generated on demand — `make libcatalogue`, not
committed) state of the loft distribution — core (version + binary sha) + libraries + applications built with loft ·
[LIBRARY_BRANCHES.md](doc/claude/LIBRARY_BRANCHES.md) in-flight (unmerged) lib branches ·
[PACKAGES.md](doc/claude/PACKAGES.md) format/targets · [PKG_REGISTRY.md](doc/claude/PKG_REGISTRY.md) registry MVP ·
[LIBRARY_AUTHORING.md](doc/claude/LIBRARY_AUTHORING.md) / [LIBRARY_CHECKLIST.md](doc/claude/LIBRARY_CHECKLIST.md) ·
[REGISTRY_SUBMIT.md](doc/claude/REGISTRY_SUBMIT.md) / [REGISTRY_BOOTSTRAP.md](doc/claude/REGISTRY_BOOTSTRAP.md) /
[REGISTRY_RECOVERY.md](doc/claude/REGISTRY_RECOVERY.md) · [API_SURFACE.md](doc/claude/API_SURFACE.md) ·
publishing is the **loft-ship skill** (touch-gated signing). REPL: [REPL.md](doc/claude/REPL.md).

**Process / issues / release:** [DEVELOPMENT.md](doc/claude/DEVELOPMENT.md) workflow ·
[ISSUE_TRACKING.md](doc/claude/ISSUE_TRACKING.md) (open→Issues, closed→[PROBLEMS.md](doc/claude/PROBLEMS.md)) ·
[.github/LABELS.md](.github/LABELS.md) · [RELEASE.md](doc/claude/RELEASE.md) · [COMPATIBILITY.md](doc/claude/COMPATIBILITY.md) (the breaking-change policy, @PLN102 arc A) · [MOVING.md](doc/claude/MOVING.md) ·
[CHANGELOG.md](CHANGELOG.md) / [CHANGELOG_TECHNICAL.md](doc/claude/CHANGELOG_TECHNICAL.md) ·
[DOC.md](doc/claude/DOC.md) · [LAVITION.md](doc/claude/LAVITION.md) · [PROMPTS.md](doc/PROMPTS.md).

**Skills** (`.claude/skills/`): `loft-write` (.loft authoring) · `loft-debug` (runtime crashes) ·
`loft-test` · `loft-codegen` · `loft-ship` (library cross-target + publish) · `engineering-rigor` /
`design-protocol` (rigor) · `doc-quality` · `draw` · `loft-plan-workflow`.

## `LOFT_LOG` quick reference

Set before `cargo test` (controls `tests/dumps/*.txt`; also works with `cargo run` → stderr):
`full` (default: IR+bytecode+exec+slots) · `static` (IR+bytecode only) · `minimal` (exec trace) ·
`crash_tail:N` (last N lines, flushed on panic) · `fn:<name>` · `variables` · `ref_debug` ·
`bridging` · `all_fns` · `type_timeline:<var>` (every write to a variable's type, naming the
SOURCE LINE; `LOFT_TIMELINE_BT=1` adds the stack). It traces deps being REMOVED
(`make_independent`) as well as added — without that half it showed a borrow being created
and never promoted to an owner, so a container-destroying free had to be hunted by reading
every strip site by hand (@PLN130 F1). DbRef dumps tune via `LOFT_DUMP_DEPTH` (2),
`LOFT_DUMP_ELEMENTS` (8). Separately, **`LOFT_VAR_TABLE=<fn>`** prints that function's
variable table with every type dep resolved to `name(index)` plus its ownership flags —
reach for it when a borrow points somewhere impossible, because the IR dump names variables
without numbering them and a code/table desync then reads as one consistent story (loft#666).
For a `--native` wrong-type fault — a sized `f#read` answering null, a keyed lookup naming a
type the program never used — reach for **`LOFT_STRICT_SCHEMA_IDS=1`**: generated `init()`
REPLAYS the parse-time type order, so one type created a position early renames every id
after it, and this makes that drift fatal instead of a report (loft#739, NATIVE.md §
Architecture). `LOFT_TRACE_MINT=1` is its companion — it names the lookup that minted the
extra type. Full API: [TESTING.md § LogConfig](doc/claude/TESTING.md), [DEBUG.md](doc/claude/DEBUG.md).

**Two diagnostic tiers.** `warning` GATES a library's CI (`LOFT_DENY_WARNINGS=1`);
`advice` never does and has no deny switch. The rule: **a diagnostic gates if and only if
ignoring it can produce a wrong result** — lost writes, char/byte index confusion,
null-into-non-null gate; deprecations, perf notes and spellings advise. The split exists
because one tier made the compat doctrine self-contradictory: `not null` is a deliberate
no-op kept parseable so unrepublished libs load, yet it hard-failed those libs' own CI.
Renders as `advice:`, LSP severity Hint; `@EXPECT_WARNING` and `Test::advice()` match it.

**Error rendering (@PLN28):** `LOFT_ERRORS=pretty|compact` (or `--errors=…`) picks the
user renderer — `pretty` (default: `file:line:col` + source line + caret) vs `compact`
(single line; the test harness pins this). Diagnostic toggles (default-on opt-outs, except
the last two which are opt-in): `LOFT_NO_WARN_RUNTIME` (undefended-fault-site warning) ·
`LOFT_NO_HINT_NOT_NULL` (`not null` field hint) · `LOFT_FORMAT_BARE_NULL` (drop the `(reason)`
suffix on `null`) · `LOFT_NO_DEAD_STORES` (@PLN107 dead-store lint: a copy mutated but never
read, e.g. `d = self.data; d[i]=x` where the bind COPIES so the write is lost — a `len(d)`
BOUND GUARD does not count as reading it, since a length cannot witness an element write;
that hole made the lint silent on `if i < len(d) { d[i]=x }`, the exact shape the `v[i]`
may-be-null warning asks for, and the published `graphics` canvas shipped every drawing
primitive as a no-op through it) ·
`LOFT_NO_DOUBLE_MOVE` (@PLN139 stage G: one droppable handed to TWO owners — `s1 = S{h:c};
s2 = S{h:c}` — where each owner's death releases what it owns, so the resource is released
twice. Counts hand-offs per source with the SAME predicate that suppresses the source's own
drop, so lint and mechanism cannot drift. `warning` because ignoring it produces a wrong
result; therefore an UNDER-approximation — silent across opposite `if` arms, a reassignment
between the hand-offs, and a terminator, and blind to the iteration count of a loop) ·
`LOFT_NO_LOST_TEMP_WRITE` (loft#894, the second `lost-write` shape: a call writing through a
by-value struct parameter GIVEN a value returned by another call — `hurt(first(s), 10.0)`
writes a copy that is freed at the end of the statement, while `hurt(s.es[0] ?? E{}, 10.0)`
lands, and nothing at the call site said which. Needs BOTH facts to meet: the callee writes
through that parameter (read off its own body) and the argument copies a place the caller
can still REACH (read off the return type's deps) — the second is what keeps
`hurt(fresh(), …)` and the write-then-return builder idiom quiet) ·
`LOFT_NO_STEER` (@PLN102 arc C recommended-idiom channel: a call FROM OWNED source to a
`#superseded "Y"` symbol warns *"`X` is superseded — use `Y`"* + a CI fold-lint; inert until a
symbol is marked — see [COMPATIBILITY.md § Folding](doc/claude/COMPATIBILITY.md)) ·
`LOFT_NO_PARAM_COUNT` (≥8 REQUIRED parameters — defaulted and compiler-hidden ones
excluded; separate from complexity because a caller's burden and a reader's burden have
different fixes: a struct vs an extracted function) · `LOFT_NO_DEFAULT_HINT` (≥2 trailing
booleans with no default — advertises default parameters, which are under-used and free to
adopt: adding a default is additive, so existing callers keep working) ·
`LOFT_NO_OMITTED_FIELD` (loft#914 `omitted-field-zero` ADVICE: a struct literal that names
SOME fields and leaves another out — the omitted one takes its type's zero and nothing in the
declaration chose it, which bites where zero is a meaningful value of the field's domain
(dryopea's palette index wanted `-1`; `0` is the entry that erases). Advertises the DECLARED
FIELD DEFAULT (`palette_pick: integer = -1`), the cure that already exists and was simply
undiscoverable. `advice`, not `warning`: the zero is documented behaviour, so ignoring it
cannot produce a result the language did not promise. Quiet on a field with a declared
default, on a NULLABLE field (absence is a value it holds), and on a bare `S {}` — that asks
for the whole default record; the ambiguity is only in the PARTIAL literal) ·
`LOFT_NO_COMPLEXITY` (function-complexity ADVICE: cognitive complexity ≥ 40 — a
construct costs `1 + nesting`, so 8 sequential `if`s cost 8, 3 nested cost 6, a flat
`match` costs 1 whatever its arm count; counted at PARSE time because the IR is
post-desugar and would charge `??` and `for` as branches the author never wrote;
names the deepest-nesting line, since that is where a split pays) ·
`LOFT_NO_STRICT_INDEX_TEXT` (@PLN110 3a text strict-index units lint: warns on
`for i in 0..len(s) { s[i] }` AND `{ s.byte_at(i) }`, incl. via a local (`n = len(s); 0..n`) —
`len(text)` is a CHARACTER count but both reads are byte-indexed, so the loop truncates
multi-byte text silently (the `cbor` encoder shipped this); advisory, use `for c in s` or
`0..size(s)`) ·
`LOFT_LINT_STRICT_INDEX` (**opt-in**, @PLN102 case-D audit: warns where a for-loop iter var
bounded by `len(<one vector>)` indexes a DIFFERENT vector — `for i in 0..len(v) { w[i] }` types
non-null yet reads C80-null on overrun; advisory, the type is unchanged) ·
`LOFT_DEV_SOFT_HALT` (**opt-in**: demote dev raises to log-and-continue so one run surfaces every fault).

**Profiling (@PLN140, all opt-in, all `--interpret`):** `LOFT_PROFILE=<ops>` samples the loft
call stack — hot FUNCTION, hot LINE, hot PATH (default one sample per 1024 ops; the op counter
picks *when*, a wall clock says *how much*, and the period is JITTERED because a fixed one
samples a single phase of a periodic program and reports it as the whole) ·
`LOFT_ALLOC_SITES=1` ranks live store BYTES by the loft line that allocated them, captured at
the run's PEAK rather than at exit · `LOFT_ALLOC_PATHS=<ops>` adds the call paths that reached
each allocation. `LOFT_PROFILE` / `LOFT_ALLOC_PATHS` also cover **test runs** (`loft test`,
`--tests`), merged into ONE report keyed by resolved `function` + `file:line` — each test
compiles its own bytecode, so positions cannot be merged, only labels (loft#860).
`LOFT_ALLOC_SITES` is program-only and says so under a suite instead of going quiet.
**A NATIVE run is not sampled** — and the default backend IS native, so a bare
`LOFT_PROFILE=1 loft p.loft` announces that rather than exiting empty (loft#865).
**A `use`d library is a cdylib the sampler cannot enter**: its functions cannot appear
and their time lands on the CALLING line, so a library doing the work reads as a hot
caller — one probe inverted from `100 % app_bit` to `99.5 % lib_grind` under
`LOFT_NO_NATIVE_LIBS=1`. The report says so whenever a library was called.
Prefer `make profile`, which picks the instrument. Off costs nothing (the
sampler rides the existing per-op debug branch); armed costs +7–11 %. PERFORMANCE.md § Profiling.
