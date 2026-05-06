<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# `par(...)` — meetup slides

Two-sheet companion to [PAR_PRESENTATION.md](PAR_PRESENTATION.md).
~15 minutes total: ~6 min on Sheet 1 (what we built), ~9 min on
Sheet 2 (how we got there together).

---

## Sheet 1 — The same parallel reduction in three languages

> Sum of squares for `1..=1000`, dispatched across 4 workers.
> Same algorithm, three surfaces.

### loft — `par(...)` is a control construct

```loft
fn square(x: const integer) -> integer { x * x }

fn main() {
    items: vector<integer> = [];
    for i in 1..1001 { items += [i]; }

    total = 0;
    for x in items par(r = square(x), 4) {
        total += r;
    }
    println("sum of squares: {total}");
}
```

- Worker fn is **named** and type-checked.
- Read-only against parent state — **enforced at compile time.**
- Thread count explicit; result order is "completion", not input.

### Java — parallelism is a stream property

```java
long total = IntStream.rangeClosed(1, 1000)
                      .parallel()
                      .mapToLong(x -> (long) x * x)
                      .sum();
```

- One pipeline, one `.parallel()` flag.
- Thread count implicit (`ForkJoinPool.commonPool()`).
- No notion of a per-element worker call site.

### Go — no parallel-for; hand-roll it

```go
partials := make([]int64, workers)
var wg sync.WaitGroup
chunk := n / workers
for t := 0; t < workers; t++ {
    wg.Add(1)
    go func(t int) {
        defer wg.Done()
        lo, hi := t*chunk, (t+1)*chunk
        if t == workers-1 { hi = n }
        var sum int64
        for _, x := range items[lo:hi] {
            sum += int64(x) * int64(x)
        }
        partials[t] = sum
    }(t)
}
wg.Wait()
var total int64
for _, p := range partials { total += p }
```

- Goroutines + `WaitGroup` + manual chunking + per-worker partials + final combine.
- Type system does not enforce per-worker isolation.

### Take-away

| Language | What you write | What you cannot express |
|---|---|---|
| loft  | one clause | nothing — the safety rule **is** the type system |
| Java  | one method | thread count, per-worker structure |
| Go    | a hand-rolled scaffold | concise dispatch |

Loft's surface is closer to Java's intent, with Go's explicit thread
count and a named worker.  **The work of plan-06 is making the
runtime behind that one clause as small as Go's hand-rolled
version.**

---

## Sheet 2 — How we got there: timeline + collaboration patterns

### Timeline at a glance

```
2026-03-15  Initial commit — par(...) already shipping
            3 native fns × 4 getters × 6 runtime variants × 2 user surfaces
            (par + par_light)

2026-04-25  Plan-06 opens.  In one day:
            characterisation suite + bench + baseline + DESIGN.md
            + redesign around "everything is a store"
            + read-only-parent as language rule + drop input-order

2026-04-29  Spine reorder.  Realisation:
            the materialised result vector is the source of complexity.
            Add phase 10 = "drop it entirely".  In 24 h:
            warning → error.  Test corpus already streaming-only.

2026-04-30  ARC.md replaces spine.  -565 LOC in step A1.

2026-05-04  T1.8a unblocks par-tuple canaries.
            Plan now reframed as a structured fuzz —
            14+ P-issues surfaced at the type × codegen × par seam.
```

### Four collaboration patterns we kept tripping over

#### 1. Drift between "the plan" and "what's next"

Three document layouts in eight days:

```
Phases (by topic) → Spine (by complexity) → ARC (scope-locked PRs)
```

Each rewrite happened the moment "what should I work on next?" had
no clear answer.  The user's correction every time was the same:
**"the doc is for deciding, not for accounting."**

> Lesson: a planning doc Claude can keep adding to is a planning
> doc that stops telling you what to do.  Hard scope locks
> (ARC.md: "OPEN / IN-FLIGHT / DONE — no partial-DONE") force
> the next decision to be visible.

#### 2. Design from spec vs. read the actual error

T1.8a (the tuple-return prerequisite) was originally scoped at
~200 LoC: new `Value::ReturnTuple` IR variant, new `OpReturnTuple`
opcode, caller-pre-allocated slot.

Then the user said: **run `--native-emit` first, read the failing
output, *then* design.**  The fix was 30 LoC of type-context
routing.

> Lesson saved to memory as "actual-error survey": bug-fix phases
> must run the failing tool and read the output **before** writing
> implementation steps.  The symptom usually points at the wrong
> layer.

#### 3. The bug-hunt reframe

Plan-06's headline metric was "~1100 LOC retired."  Through April
the canaries kept surfacing P-issues — P188, P189(a-d), P191, P195,
P196, P198, P199, P200, P201 — at the type × codegen × par seam.

The user's reframe: **"as long as canaries keep firing, plan-06's
per-day yield is high — even if no LOC retire that day."**

The plan was no longer "delete the marshalling code"; it was
"structured fuzz of an interaction surface that no doc-test will
hit."

> Lesson saved to memory as "proactive bug hunting": extend
> libraries to find compiler bugs.  File P-IDs first, fix when
> reasonable.

#### 4. Scope discipline against well-meaning shortcuts

Three patterns of shortcut Claude proposed and the user shut down:

| Shortcut | Why tempting | User's rule |
|---|---|---|
| Mark a known-failing canary `#[ignore]` to ship the PR | Unblocks the merge | **No `EXPECT_FAIL` for PR-blocking bugs.**  Fix it or don't ship. |
| Branch off `main` while the current branch has unmerged work | Cleaner history | **Branch-after-PR only.**  New branch loses access to in-branch progress. |
| Accept a small regression to land a refactor sooner | Trade time for time | **Zero regression tolerance.**  The proper fix takes as long as it takes. |

> Lesson: a 9-year-old codebase is not optimising for sprint
> velocity.  The collaboration shape is "long-horizon foundations,"
> not "ship MVP, fix later."

### What this looked like at the artifact level

```
CLAUDE.md           — ground rules; never lets Claude branch/push without ask
plans/06-typed-par/
    README.md       — phases by topic (historical)
    PRIORITY.md     — spine (historical)
    ARC.md          — scope-locked steps (current source of truth)
    DESIGN.md       — cross-cutting decisions referenced by every phase
PROBLEMS.md         — every P-issue surfaced during the work
MEMORY.md           — the user's corrections to Claude, persistent across sessions
```

Each layer has one job; the moment a layer started doing two jobs
(spine doing both planning *and* accounting), it got split.

### One sentence

> The interesting part of this collaboration was not the par
> redesign.  It was discovering what kind of structure a long-running
> Claude session needs around it so that the redesign can keep
> moving without re-deriving its own plan every week.
