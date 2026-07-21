# H2 probe corpus

Two matrices over the same defect, varying **different axes**.  Run both.

| corpus | harness | varies | holds fixed |
|---|---|---|---|
| `p1`–`p8` | `./run.sh` | the **call-site shape** (loop / temp / literal / via-local / no-temp) | the callee — every cell calls the same struct-literal `mk` |
| `r1`–`r12` | `./run-callee.sh` | the **callee's return** — what STORE it hands back | the call site — every cell is `out += [f(…)]` |

## Why the second matrix exists

The `p` corpus found H2 and is what the root cause was read off.  But it holds the callee
fixed, and the fix's correctness depends entirely on what the callee returns: the
value-before-slot patch went **green on all 8 p-cells while leaking**, and the leak only
surfaced 224 seconds later in the full suite.  That is a matrix aimed down the wrong axis
for the question being asked.  `run-callee.sh` reproduces the same leak in 3 seconds as
`r10_orelse_fresh_loop`.

## The axis

| cells | callee returns | static verdict |
|---|---|---|
| `r1`,`r2` | its NRVO return buffer (a struct literal) | `Owned` |
| `r3`,`r4` | another call's store, no literal of its own | `Owned` |
| `r5`,`r6` | retbuf on one path, another call's store on the other | `Owned` |
| `r7`,`r8` | a borrowed view into an argument | `Join(base=…)` |
| `r9`–`r12` | **borrow OR fresh, decided at runtime** (`t[i] ?? m_none()`) | `Join(base=t)` |

`r9`–`r12` are the load-bearing ones: that runtime choice is what `scopes.rs` says cannot
be resolved statically (its `map_from_json` example), and it is the shape
`tests/use_analysis.rs::ELEM_SRC` uses.  Note `r1`–`r6` all carry the SAME static verdict
(`Owned`) yet behave differently — the classification does not separate them, only running
them does.

Each cell is also crossed with loop vs straight-line, because H2 needs a loop while the
`r5`/`r6` leak does not — a distinction invisible if only one flow is probed.

## What each cell asserts

**Value AND length AND leak, on BOTH backends.**  Each alone is blind to the others: H2
is a value fault with correct length, the `r5` finding is a leak with correct values, and
a delivery that doubled elements would read as leak-free.  H2 is interpreter-only, so a
single-backend run reports green on the broken one.

Expectation is `1 2 3` / `len=3` for every cell by construction; `r10` is the one
deliberate exception (it forces the out-of-range arm, so `-1 -1 -1` is correct) and is
spelled out in the harness rather than left implicit.

## Findings so far

- **`r1`** — H2 itself; fixed by `../value-before-slot.patch`.
- **`r5`/`r6`** — an INDEPENDENT interpreter leak (2 stores) in the branch-returning
  callee.  Needs no loop, native is clean, and the patch neither causes nor fixes it.
  **Not yet checked against `main`** — do that before filing, per the bug-filing policy.
- **`r10`** — the patch's blocker: `M×3` leaked when the callee takes its fresh arm.
