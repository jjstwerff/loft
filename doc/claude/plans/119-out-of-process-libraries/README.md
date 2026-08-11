<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 119 — Out-of-process libraries: placement as policy, the store as the wire

## Status

Open — **arcs A and D done, arc B half done (2026-08-11)**.  A library declares
`placement = "process"` and its consumers call it unchanged, across a real
process boundary.  The parity gate is green for **every scalar shape**: each
integer width with its sign, `single`, boolean, and text in both directions
([Arc B, first half](#arc-b-first-half-as-built-2026-08-11)).  A worker that
dies is an error rather than a hang ([Arc D](#arc-d-as-built-2026-08-11)).
**Q1 answered GREEN (2026-07-24)**; **Q4 answered, then CORRECTED (2026-08-11)**
— the crossing is affordable and the handshake is right, but the 130 ns was a
bare wire ping and a real call cost 4.7 µs, almost all of it a span table being
deep-cloned on every entry into loft; fixed, and a placed call is now ~1 µs
([Q4](#open-design-questions)).

**Next: arc B's second half** — structs, vectors and references, i.e. the arena
— and arc C rides on it.  This is a DEPARTURE from the phase ordering below,
which put arc C first, and the reason is worth stating: arc C proves ownership
across the boundary, and **nothing that crosses today owns anything**.  Every
value is copied through `host::Value`, so the @PLN94 oracle would report
agreement by having nothing to disagree about — the shape
[absent-warning-is-not-a-pass](../../STABILITY_METHOD.md) warns against.  Arc C
becomes real the moment a reference crosses, which is arc B's second half.
Opened 2026-07-24 from the question "does loft have a safe way to spawn
sub-processes?".  It does not, and the answer is deliberately not to add one:
`lib_plans/67-process/`'s `run(cmd, args) -> {stdout, stderr, code}` is
superseded by this plan, and a general `run()` is declined in
[DESIGN_DECISIONS.md C101](../../DESIGN_DECISIONS.md).  The substrate this plan
assembles is already in the tree (mmap-backed stores, position-independent
`DbRef`, the @PLN97 layout contract, @PLN86 admission); what is missing is the
boundary marshal, the ownership rules across it, and the proof.

**Q1 (text residency) is settled, and it was NOT the hazard this plan first
claimed.**  A probe wrote a record via `store_persist_bind` and read it back in a
SEPARATE PROCESS: short text, a 300-char literal (past the 256-byte threshold
where a constant stops being inlined in bytecode), runtime-concatenated text,
multi-byte UTF-8, and a `vector<text>` all crossed intact — on both the
mmap-adopt (`store_persist_bind`) and heap-copy (`store_load`) paths, and from an
interpreter writer to a `--native` reader.  A canary proved the harness reports
FAIL, and an absent-key control proved the reader invents nothing.  Text is
store-resident, therefore mmap-resident.  Guard:
`tests/scripts/store_handoff_residency.loft` driven by
`tests/store_persist_loft.rs::handoff_text_residency_*`.

The original Q1 conflated two paths.  The `vector<text>` refusal in
`Stores::is_copyable_field` (`src/database/allocation.rs:2903`, pinned by
`tests/scripts/store_load_vectext_refuse.loft`) belongs to the **paged
working-set loader**, which RELOCATES a matched entry's heap graph into a
DIFFERENT store — there each element's string pointer would dangle.  A shared
arena never relocates: both sides map the same store.  That refusal therefore
does not bound this design, and the "no serialization is aspirational for
text-heavy APIs" caveat is withdrawn.

## Goal

Let a loft library run in its own process — or on another machine — addressed
through the **same typed `pub fn` interface** it has in-process, with no second
wire vocabulary and no serialization tax on the data it shares.

## The one invariant

> **A call to a library is indistinguishable — in type, effect, ownership/lifetime,
> and error behaviour — from the same call in-process.  Where it runs is deployment
> policy, not source.**

Every arc below serves this.  Its **re-assertion sites** — the places the invariant
must independently hold, and therefore the places that can disagree — are:
argument marshal, return marshal, ownership transfer (who frees), null/sentinel
propagation, effect classification, capability admission, and the leak check.
Single-home discipline applies: each of those facts gets **one** home both
placements consult, so the off-diagonal cells cannot disagree.

## Effort + design

- **Effort:** H (arcs A–C are the core; E and F ride on them)
- **Design:** ~ (partial — the boundary marshal and the ownership rules are
  hypotheses until arc B/C probes run; see [Open design questions](#open-design-questions))
- **Last touched:** 2026-07-24

## Why not a subprocess primitive

`run(cmd, args)` is a *second, weaker interface* beside the one we already have.
The library interface carries typed signatures, structs/enums/vectors/tuples,
methods, coroutines, effects, and capability admission; a `{stdout, stderr, code}`
triple carries bytes and an exit status.  Adding it would mean every consumer
re-parses text that loft already knows how to type.

So an external command lives **inside** a vetted library — `lib/git` exposes
`git::log(range) -> vector<Commit>`, and the `execve` is an implementation detail
sealed behind that contract with its own capability token (`git#read`).  This also
removes the injection surface by construction, rather than by the
argument-vector-not-a-string rule `67-process` had to invent.

## Shape — what crosses is data, not orders

| Concern | Mechanism | Already in tree |
|---|---|---|
| Interface | the library's `pub fn` signatures | — |
| Placement | `[lib] placement = "inproc" \| "process" \| "remote"` in `loft.toml` | new (arc A) |
| Wire | a shared **mmap-backed store** as the call arena | `Store { ptr, file: Option<MmapStorage> }` (`src/store.rs:129-135`), `open` / `open_durable` |
| Pointer portability | `DbRef = (store_nr, rec, pos)`, **no raw pointer** — survives mapping at another address; only `store_nr` is translated (O(1)/ref, zero page copies) | `src/keys.rs` |
| Schema safety | `.dschema` **layout-identity gate** — refuses a store whose layout differs from the loading program's type instead of misreading foreign bytes; `store_verify` backstop | @PLN97 arc G, `src/schema_sidecar.rs`, `src/native.rs:1252`, [formal/layout.md](../../formal/layout.md) |
| Remote = same mechanism | working-set page loads from a file **or an `http(s)://` Range server** | `src/paged_reader.rs` |
| Concurrency | **single-writer per store** (the free-space LLRB tree + `needs_coalesce` are single-writer); readers map read-only | `read_only` / `free_protected` locks, `clone_locked` / `borrow_locked_for_light_worker` |
| Staleness | the store's monotonic mutation counter (bumped on `claim`/`resize`/`delete`) is already the epoch a cross-process reader needs | CO1.9/S28, `src/store.rs:166-169` |
| Transactional writes (later) | journal snapshot — the transactional world [SANDBOX.md](../../SANDBOX.md) S7 wanted and dropped | `src/database/journal.rs::snapshot` |
| Admission | it's a library: gated by name (`allow_libs`) or capability (`group#right`), deny-by-default | @PLN86, `src/sandbox.rs` |
| Typed entry point | `Program::call(func, args) -> Value` already marshals typed args into a loft fn | `src/host.rs:188` |

Two consequences worth stating explicitly:

- **The mmap axis is the point, not an optimisation.** Because the arena is a
  mapped file under the layout contract, *same process*, *another process*,
  *another machine*, and *a later run of the same program* become one mechanism at
  four latencies.  When full database access lands, an out-of-process library is
  not a new concept — it is a second attached reader, and the library API is its
  query vocabulary.
- **`SANDBOX.md` gains a row.**  Its untrusted tier is documented as
  *wasm-isolated, marshalled, slower*.  Out-of-process + mmap + layout gate is
  isolated **and** direct-data, because the boundary is a page table rather than a
  marshalling step.  Fault isolation is the upgrade an in-process library cannot
  offer: a crashed worker aborts *the call* as a loft error and cannot corrupt the
  caller's stores.

**Expressiveness kept:** streaming is a **coroutine** yielding typed values (not
`read_line() -> text` at EOF-null); state persists because the worker owns its
stores across calls; errors are loft errors; and the same source runs in-proc,
out-of-proc, or remote by flipping one policy line.

## Composition matrix — Stage A

This plan does **not** invent a matrix.  It multiplies the existing axes
([README § The composition axes](../README.md#the-composition-axes--the-dimensions-a-matrix-varies))
by one new axis:

> **Axis P — placement:** `inproc` · `process` · `remote`.

A cell is green when the value round-trips across the boundary with identical
value, length, ownership, and leak state.  Probes live in `probes/`, written on
`--interpret` first, and graduate to `tests/scripts/119-placement.loft`.

| Axis | Cells to cross with P | Why it can diverge |
|---|---|---|
| **1 Type-kind** | wide scalar · **narrow scalar** (`i8`/`i16`/`u8`/`u16`) · **text** · struct · enum · vector · tuple · closure/fn-ref · the **null** of each | narrow strides and text are the two representations that are not plain in-store words |
| **2 Construction path** | literal · comprehension · fn return · append · copy · default-init | an appended vector may live in a different store than a literal one |
| **3 Storage context** | local · struct field · vector element · global · const · argument | const lives in `CONST_STORE`, which the callee does not own |
| **5 Nesting depth** | 1 / 2 / **3+** | `vector<vector<T>>` is the shape @PLN97's relocator refuses today |
| **6 Null / sentinel** | per-type null across the boundary | a null `DbRef` must not become a *valid* rec-id after `store_nr` translation |
| **7 Backend** | `--interpret` × `--native` | the marshal is emitted code on one side and interpreted on the other |

**The text row is GREEN as of 2026-07-24** — every text shape, `vector<text>`
included, survives a whole-store handoff between processes (see Status).  The
matrix still has to run it per placement: residency proves the bytes are in the
arena, not that a `remote` placement's page fetch delivers them.  And the
discipline that found the error stands — a matrix that runs wide scalars and
skips `vector<text>` would pass on the axes that cannot break, exactly the
plan-58 failure.

## Sub-arcs

| Item | Source | Status |
|---|---|---|
| **A** — placement declaration + attach handshake (`store_nr` translation, epoch check) | this README | **Done 2026-08-11** (scalars + text arguments; see below) |
| **B** — boundary marshal: arena residency for every value reachable from an argument or return | this README + Q1 | **Scalars done 2026-08-11** — every integer width with its sign, `single`, and text RETURNS ([Arc B, first half](#arc-b-first-half-as-built-2026-08-11)). Structs, vectors and references — the arena proper — remain |
| **C** — ownership + lifetime across the boundary, proven with the @PLN94 oracle | this README + Q2 | Open — and still **vacuous until arc B's second half**: nothing that crosses today carries ownership, so the oracle would agree by having nothing to disagree about |
| **D** — fault isolation: worker death → typed loft error, caller stores provably intact | this README | **Done 2026-08-11** — a killed worker is an error, not a hang ([Arc D as built](#arc-d-as-built-2026-08-11)) |
| **E** — `placement = "remote"` over the existing paged / Range reader | @PLN97 arc G | Open — blocked-ish on [#632](https://github.com/loft-lang/loft/issues/632): the paged loaders silently refuse a **field-declared** collection (the store's `known_type` is the wrapper struct), and the refusal is indistinguishable from "key absent" |
| **F** — consumers: `lib/git` first, then the engine_host wire | [lib_plans/67-process](../../lib_plans/67-process/README.md) | Open |

## Arc A as built (2026-08-11)

A library writes one line in its own `loft.toml`:

```toml
[library]
placement = "process"
```

and its consumers are unchanged — `use maths;` then `add(2, 3)`. The call
compiles to an `OpStaticCall`, whose stub is replaced by a dispatcher that reads
the arguments off the interpreter stack, crosses a shared mmap to a worker
process holding the library, and writes the answer back.

| Piece | Where |
|---|---|
| Declaration + validation (portable) | `src/lib_placement.rs` |
| The wire: mapping, handshake, frame codec, both ends | `src/lib_placement/wire.rs` |
| Marking + the interpreter dispatcher | `src/lib_placement/dispatch.rs` |
| Worker entry (`--lib-worker`, internal) | `src/main.rs` |
| **The gate** — one consumer, one library, both placements | `tests/placement_parity.rs` |
| Transport in isolation (`ping` crosses, shapes survive, faults) | `tests/placement_worker.rs` |

Three things worth carrying forward:

- **The boundary reuses `host::Value`.** The worker loads the library as a
  `host::Program` and serves calls through the same typed marshaller a Rust
  caller uses. That is the plan's own "no second wire vocabulary" rule applied to
  its own implementation, and it is why arc A is small.
- **A signature the wire cannot carry is not marked**, so it runs in-process —
  byte-identically, the same fallback an uncompilable native library takes.
  Nothing becomes a call that fails later. Arc A carries integer-family, boolean
  and text ARGUMENTS, and void / integer-family / boolean RETURNS.
- **A text return is excluded on purpose.** It travels the interpreter's
  dest-buffer protocol (`n_set_bridge_dest`), which codegen emits only for a
  cdylib call; approximating it would return a wrong value rather than refuse.
  It belongs with arc B, next to `single`, structs, vectors and references.

**The gate isolates ONE axis.** Left alone, the in-process side auto-compiles the
library to a cdylib while the placed side does not, so the two runs would differ
in whether a native build ran — and that build's chatter reads as a placement
difference when it is nothing of the kind. Both sides are pinned to the
interpreter (`LOFT_NO_NATIVE_LIBS=1`) so placement is the only thing that
changed. The gate carries its own control (`the_gate_can_fail`): it compares two
deliberately DIFFERENT libraries and requires the comparison to notice, because a
parity gate that cannot report a difference passes forever.

**Still honest about what is unproven.** `store_nr` translation and the epoch
check are exchanged at handshake but nothing reads them yet — arc A sends no
references, so there is nothing to translate. They are written at attach anyway
because adding them when arc B needs them would be a protocol change that
silently mismatches a running worker. Do not read the handshake fields as
evidence that translation works.

**Three known limits, all shaped rather than accidental:**

1. **Calls to a placed library serialise.** The wire has one request slot, so
   the dispatcher holds a lock across the crossing — correctness, not caution,
   but it makes a placed library a poor fit for a hot `par` arm. The plan's
   single-writer discipline is where this gets revisited.
2. **The transport is Linux-only** (it is built on `futex`). Elsewhere a placed
   library runs in-process, which by the invariant is the same program;
   `LOFT_REQUIRE_PLACEMENT=1` makes the lost isolation an error instead.
3. **`--native` does not place at all** — it compiles the library's body into
   the program binary. Same treatment as (2) since arc D: no worker is started,
   and `LOFT_REQUIRE_PLACEMENT=1` refuses and says why. See
   [The gate](#the-gate).

## Arc B, first half, as built (2026-08-11)

Every scalar shape now crosses: each integer width **with its sign**, `single`
both ways, and a text RETURN.  What is left of arc B is the arena — structs,
vectors, references — which is where "no serialization tax" is actually won or
lost.

The half that landed was mostly not new mechanism.  It was **three bugs in
shared code**, each of which had been sitting in a path older than this plan:

1. **A narrow integer argument read the previous call's bytes.**  `loft::host`'s
   marshal sized an integer by its *storage* width where the stack ABI needed its
   *cell* width — eight bytes for every integer alias.  The stack steps in 8-byte
   units, so the short write did not shorten the frame and nothing crashed: it
   filled byte 0 and left the other seven holding the last call's data.  After a
   call carrying `0x0F0F0F0F0F0F0F0F`, `f(1)` on a `u8` parameter answered
   `0x0F0F0F0F0F0F0F01`.  The return direction was wrong in reverse — read narrow
   and zero-extended, so `-1` as an `i8` came back as `255`.  This was never
   placement-specific; it broke `loft::host` for any Rust caller.
2. **A library that warns could not be placed at all.**  `parse_dir` refused a
   directory whose parse reported *anything*, so one `never-read` warning made
   the consumer exit 1 placed and 0 in-process — placement deciding whether the
   program ran.
3. **Entering loft deep-cloned the whole fault-site span table.**  Invisible for
   a program entered once, and 4.4 µs of every 4.7 µs `host` call for one entered
   in a loop.  See [Q4](#open-design-questions), which this corrects.

The text return is the one piece that needed reading the emitted code rather
than reasoning about it.  A text answer does not come back on the stack, and the
call site picks between **two** conventions: assigned to a text variable, codegen
stashes a destination record and expects nothing pushed; everywhere else it
passes the hidden `&text` work buffer a promoted text return carries and expects
a `Str` over it.  Both were already being emitted for a placed function — the
routing keys on a native symbol plus a text return, exactly what marking leaves
behind — so the dispatcher's job was to read which one the call site chose, not
to impose one.  Two things only the bytecode said:

- The hidden work buffer is **pushed like any other argument** and was not being
  popped.  That does not fail where it happens; the cell stays behind and shifts
  every later frame.
- A text return the compiler never promoted — a constant, `fn version() -> text
  { "1.0" }` — carries no work buffer at all, so the caller offers nowhere for
  the answer to live and a `Str` over the worker's own String would dangle.  Such
  a function **is not placed** and runs in-process, the same fallback every other
  unsupported signature takes.  Making it uniform means promoting that return to
  a retbuf (@PLN104's transform, which today considers only the main program's
  own definitions) — the one loose end of this half.

## Arc D as built (2026-08-11)

A worker that dies is now the caller's error rather than its hang.

`Worker::dead` existed and was read on entry to every call, which reads as the
death being handled.  Nothing ever set it, because the wait had no way to
notice: a caller waits on a futex word in the shared mapping, and `FUTEX_WAIT`
does not fail when the process that would write that word is gone — it waits
forever.  Killing a worker mid-call hung the caller with no timeout and no
output, which looks exactly like the program itself having stopped.

The caller's wait now sleeps in bounded steps and looks at the child when one
expires — `try_wait`, not a signal probe, because an unreaped worker is a zombie:
a live pid that answers `kill(0)` and will never serve another call.  The
handshake takes the same path, so a worker that crashes while *loading* a library
is caught too, where before it hung at startup.  The worker's own wait stays
untimed; `PR_SET_PDEATHSIG` already tells it the caller is gone.

Only a wait that has spun past its budget ever sleeps, so a busy exchange does
not pay for this — measured, 200k placed calls cost the same either way.  Guard:
`tests/placement_worker.rs::a_worker_killed_mid_call_is_an_error_not_a_hang`,
whose control is the removal of the poll (that version times out).

**What arc D does NOT yet prove** is the other half of its own row: *caller
stores provably intact*.  The structural argument is strong — the worker has its
own address space and only values are copied in and out — but "provably" wants
the @PLN94 oracle, and that belongs with arc C.

## Phase ordering

1. ~~**Q1 probe before anything else**~~ — **DONE 2026-07-24, green** (Status).
   Residency is proven for every text shape across a real process handoff, so
   arc B is unblocked and starts from "build arguments in the arena", not from
   "can text cross at all".
2. ~~**Arc A** — attach handshake and `store_nr` translation~~ — **DONE
   2026-08-11** ([Arc A as built](#arc-a-as-built-2026-08-11)).  `fn ping() ->
   integer` crosses the boundary, and the parity gate holds for scalar and
   text-argument calls.  The epoch/`store_nr` words are exchanged but unread
   until references cross.
3. ~~**Arc C** before arc B~~ — **REORDERED 2026-08-11.**  The original reason was
   that "the leak half of the parity gate cannot pass before this".  Running the
   gate settled it the other way: the leak half passes today, because nothing
   that crosses owns anything — every value is copied through `host::Value`.  So
   arc C run now would be an oracle agreeing with itself, and it moves to after
   the arena, where a reference finally crosses and there is an ownership
   question to answer.
4. ~~**Arc B**, first half~~ — **DONE 2026-08-11**: every scalar shape, both
   directions ([Arc B, first half](#arc-b-first-half-as-built-2026-08-11)).
5. **Arc B, second half** — the arena: structs, vectors, references, and the
   nesting depths of Stage A's matrix.  This is where "no serialization tax" is
   won or lost, and where the epoch and `store_nr` words finally get a reader.
6. **Arc C** — ownership, with the @PLN94 oracle run on both placements, against
   values that actually carry ownership.  Also finishes arc D's second half
   (*caller stores provably intact*), which today rests on a structural argument
   rather than the oracle.
7. ~~**Arc D**~~ — **DONE 2026-08-11** ([Arc D](#arc-d-as-built-2026-08-11)),
   taken out of order because the hang it fixes was reachable from arc A: a
   killed worker hung the caller with no timeout and no output.  Still
   interpreter-only; `--native` is untried (see the gate note below).
8. **Arc F** — `lib/git`, which deletes `tools/viewer/refresh.sh` (~140 lines of
   bash) and removes `make index`'s "filter loft to bash-tracked files" workaround.
9. **Arc E**, then the engine_host wire (F, second half).

## The gate

One unchanged consumer + library, run under `placement = "inproc"` and
`placement = "process"`, requiring **byte-identical output** *and* **identical
`check_store_leaks` state** on `--interpret` (leak checking needs `--interpret`;
bare `loft` is native and skips it).  Any divergence falsifies the invariant.
This is the same instrument shape as the four-target parity gate in the
loft-ship skill, with placement as the axis instead of target.

**What the gate proves, and what it does not.**  Its VALUE half is proven live:
`the_gate_can_fail` runs two deliberately different libraries and requires the
comparison to notice.  Its LEAK half is not, and should not be read as though it
were.  `check_store_leaks` prints to stderr and the gate compares stderr, so a
leak *would* show — but no probe has yet made that channel report on demand, so
"no leak" is corroboration rather than a proven gate.  Positive readings taken
alongside it (`LOFT_ALLOC_REPORT`, `LOFT_TEXT_TIMELINE`, `LOFT_STRICT_STORES`)
agree across placements, which is evidence of the same kind.  Making the leak
half falsifiable belongs with arc C, where a leak becomes possible.

**`--native` is outside the gate, deliberately.**  That backend compiles a
library's own body into the whole-program binary, so a placed library's calls do
not leave the process however they are marked — placement has no effect there,
and measuring confirmed it (200k calls cost the same either way, ~10× too fast
to be crossing).  Since arc D that is no longer silent: `--native` does not mark
or start workers at all (it used to start one per library and never call it),
and `LOFT_REQUIRE_PLACEMENT=1` refuses, naming `--native` as the reason, exactly
as it refuses on a platform with no transport.  Guard:
`tests/placement_parity.rs::native_does_not_place_and_says_so_when_asked_to_insist`.
Making `--native` genuinely place a library is its own arc, and it is not free:
the generated Rust would have to call the dispatcher rather than the body.

## Open design questions

1. **~~Text residency — the known-red cell~~ — ANSWERED GREEN 2026-07-24** (see
   Status).  Text, `vector<text>` included, is store-resident and crosses a
   whole-store handoff intact; the paged relocator's refusal is a different path
   and does not apply to a shared arena.  What survives is narrower, and is arc
   B's actual job: `Str{ptr,len}` is a raw Rust-heap pointer (`src/keys.rs`)
   while a value is *live in a program*, so the marshal must **construct
   arguments in the arena** — an ordinary store write, exactly what the probe's
   writer did — rather than hand the callee a pointer into the caller's Rust
   heap.  Open sub-question: for a value the caller already holds as a local, is
   that in-place (already arena-resident) or one copy at the boundary?
2. **Ownership of a returned graph.**  Hypothesis: the callee allocates in the
   arena the caller owns, and the existing `OpFreeRef` path frees it — the same
   rules, one store number.  Unproven; arc C settles it with the @PLN94 oracle.
3. **Effect classification.**  Which `ImpureCategory` (`src/data.rs:2638` — HostIo /
   Prng / Io / ParentWrite / ParCall) does a cross-placement call carry, and is a
   `process`-placed library callable from a `par` worker?  Reusing `Io` is the
   cheap answer; par-safety may force a sixth category.
4. **~~Crossing cost~~ — ANSWERED 2026-08-11, premise SURVIVES, but it dictates
   the handshake.**  Measured on x86_64 Linux (24 cores): two processes sharing
   one mmap page, bouncing a `ping()` with no body, against the same trivial
   `pub fn` called in-process (timed against the identical loop with the call
   removed; matching checksums prove the subtraction).

   | path | ns per call |
   |---|---|
   | in-process loft call, `--interpret` | 16 |
   | in-process loft call, `--native` | 51 |
   | **crossing, adaptive spin-then-sleep** | **124–138** |
   | crossing, futex wake on every call | 3900–4200 |
   | *control:* zero spin budget (forced sleep) | 5100 |

   The load-bearing finding is that **the obvious implementation is the wrong
   one by a factor of 30**.  A plain "wake the worker" futex costs ~4 µs — 77×
   the native in-process call, 246× the interpreted one — and at that price
   placement is *not* policy, because moving a library would change the shape of
   the code calling it.  An **adaptive spin-then-sleep handshake with a sleeper
   flag** — the waker pays the `FUTEX_WAKE` syscall only when the counterpart
   actually went to sleep — lands at ~130 ns, i.e. **2.4× a native in-process
   call and 8× an interpreted one**, while an idle worker still returns its
   core.  That is ≪ the work for any library call that does more than a few
   hundred nanoseconds, which is every consumer arc F names.

   So the crossing is affordable, and arc A inherits a REQUIREMENT rather than a
   free choice: the handshake is spin-then-sleep with a sleeper flag, and the
   naive wake is a performance bug, not a simpler variant.  The control row is
   what makes the shippable row trustworthy — with a zero spin budget the same
   code lands on the futex number, proving the sleep path is genuinely taken and
   the 130 ns is not a spin in disguise.

   Probe: `probes/q4_crossing.rs` + `probes/q4_inproc.loft`.  What is NOT
   measured yet: the cost of marshalling real arguments into the arena (arc B),
   which is the term that actually grows with the call's data.

   **Corrected 2026-08-11 — the wire was never the expensive part.**  Those
   numbers are a bare ping between two processes, and taking them for the cost
   of a *call* was wrong by a factor of thirty.  Measured end to end (200k calls
   to a placed `fn tick(n) -> integer`, against the identical in-process loop),
   a placed call cost **3.7 µs** — and raising the spin budget changed nothing,
   which falsified the obvious reading that the caller was sleeping.  It was not
   in the handshake at all: `loft::host::Program::call` itself cost **4.7 µs**
   with no wire involved, of which **4.4 µs was publishing the fault-site span
   table** — a `BTreeMap` deep-cloned into a fresh `Arc` on every entry into
   loft.  Building that snapshot once brought a host call to **0.47 µs** and the
   placed round trip to **~1 µs**, i.e. ~20× an interpreted in-process call.

   Two lessons worth carrying, both of which cost time here:

   - **A microbenchmark of the transport is not a measurement of the feature.**
     The 130 ns is real and the handshake design it justified is right; it was
     simply never the term that dominated.  The end-to-end number is the one that
     answers "is placement policy".
   - **The spin-budget sweep is what made this findable.**  The first story —
     "the worker's turnaround exceeds the spin budget, so every call sleeps" —
     was coherent, fitted the ~4 µs, and was wrong.  Three budgets, 2000 through
     100000, produced the same time; that flat line is what said "look
     elsewhere" before any code was changed on the strength of the story.
5. **Writer discipline under real database support.**  MVP is share-read-only +
   explicit write-back.  When full DB access lands, does the writer path become a
   journal transaction, and does that change the invariant's error behaviour?

## Cross-arc dependencies

- **@PLN97** (layout contract / working-set loader) — this plan extends the layout
  contract from data-at-rest to calls; arc E is its Range reader unchanged.
- **@PLN86** (sandbox admission) — supplies the gate; this plan adds the fault
  isolation @PLN86's compile-only decision deliberately left out.
- **@PLN94** (ownership oracle) — the instrument arc C is proven with.
- **@PLN103** (lifetime inspector) — should render "owned by which store" across a
  placement boundary, or it goes blind exactly where this plan is riskiest.

## See also

- [DESIGN_DECISIONS.md C72](../../DESIGN_DECISIONS.md) — a general `run()` declined.
- [lib_plans/67-process](../../lib_plans/67-process/README.md) — superseded; its
  consumer list becomes arc F.
- [DATABASE.md](../../DATABASE.md) — stores, `DbRef`, the working-set loader.
- [SANDBOX.md](../../SANDBOX.md) — the trilemma table this plan adds a row to.
- [OWNERSHIP_MODEL.md](../../OWNERSHIP_MODEL.md) — the deps north-star arc C must hold.
- [formal/layout.md](../../formal/layout.md) — the layout contract used as the wire.
- `tests/scripts/store_handoff_residency.loft` +
  `tests/store_persist_loft.rs::handoff_text_residency_*` — the Q1 residency
  guard (writer task → reader task, both whole-store paths, both backends).
- `tests/scripts/store_load_vectext_refuse.loft` — its counterpart: the PAGED
  loader's `vector<text>` refusal.  Read the two together; conflating them is
  what produced this plan's original wrong Q1.
- @PLN119 — <https://github.com/loft-lang/plans/issues/119> (this plan).
