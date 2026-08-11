<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 119 — Out-of-process libraries: placement as policy, the store as the wire

## Status

Open — **Q1 answered GREEN (2026-07-24)**, **Q4 answered (2026-08-11): the
crossing is affordable, and it pins the handshake to adaptive spin-then-sleep**
(see [Open design questions](#open-design-questions) 4 — the naive futex wake is
30× worse and would have made placement visible in the source).  Arc A in
progress.
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
| **A** — placement declaration + attach handshake (`store_nr` translation, epoch check) | this README | Open |
| **B** — boundary marshal: arena residency for every value reachable from an argument or return | this README + Q1 | Open — **unblocked**; residency proven, the job is to build args in the arena |
| **C** — ownership + lifetime across the boundary, proven with the @PLN94 oracle | this README + Q2 | Open |
| **D** — fault isolation: worker death → typed loft error, caller stores provably intact | this README | Open |
| **E** — `placement = "remote"` over the existing paged / Range reader | @PLN97 arc G | Open — blocked-ish on [#632](https://github.com/loft-lang/loft/issues/632): the paged loaders silently refuse a **field-declared** collection (the store's `known_type` is the wrapper struct), and the refusal is indistinguishable from "key absent" |
| **F** — consumers: `lib/git` first, then the engine_host wire | [lib_plans/67-process](../../lib_plans/67-process/README.md) | Open |

## Phase ordering

1. ~~**Q1 probe before anything else**~~ — **DONE 2026-07-24, green** (Status).
   Residency is proven for every text shape across a real process handoff, so
   arc B is unblocked and starts from "build arguments in the arena", not from
   "can text cross at all".
2. **Arc A** — attach handshake and `store_nr` translation, with the epoch check
   wired to the existing mutation counter.  A no-op library (`fn ping() -> integer`)
   crossing the boundary is the first green cell.
3. **Arc C** — ownership, with the @PLN94 oracle run on both placements; the leak
   half of the parity gate cannot pass before this.
4. **Arc B proper** — the full type-kind × placement matrix from Stage A.
5. **Arc D** — kill the worker mid-call; assert a typed error and an intact caller
   store, both backends.
6. **Arc F** — `lib/git`, which deletes `tools/viewer/refresh.sh` (~140 lines of
   bash) and removes `make index`'s "filter loft to bash-tracked files" workaround.
7. **Arc E**, then the engine_host wire (F, second half).

## The gate

One unchanged consumer + library, run under `placement = "inproc"` and
`placement = "process"`, requiring **byte-identical output** *and* **identical
`check_store_leaks` state** on `--interpret` (leak checking needs `--interpret`;
bare `loft` is native and skips it).  Any divergence falsifies the invariant.
This is the same instrument shape as the four-target parity gate in the
loft-ship skill, with placement as the axis instead of target.

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
