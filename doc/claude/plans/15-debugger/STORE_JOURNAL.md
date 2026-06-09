<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 15.J — Store change journal (the live-edit substrate)

> **Identity:** a design sub-doc of `@PLN15` (debugger). Slug `store-journal`.
> **Status:** design — **one uniform model** for every consumer (§ The model);
> `Modify` slice + the keystone `claim_at` built (`src/database/journal.rs`,
> `src/store.rs`). Load-bearing facts **confirmed** — the mutation chokepoint
> (§ Chokepoint), the relocating-grow facts (probes #5a–c; 5a falsified the naive "freed
> bytes survive" draft), and **the keystone itself (probe #6): `claim_at` is
> position-exact and the op-log replays both directions**. The predicted coalesce-corner
> falsification **landed in the fuzz pass (6c)** — `claim_at` couldn't carve a free-but-
> *fragmented* region; fixed (it now absorbs adjacent free blocks) and re-run clean over
> 12 seeds × 600 ops. The reverse-order invariant that makes revert sound is stated
> (§ The model). Freed records: *snapshot-to-blob + `claim_at`* (§ Open design points).
> Remaining build: wire `Insert`/`Free` + the resize-caller pointer marks into the
> debugger edit path (§ Phasing 3).

## Why this exists

The debugger's **live edits** are complete for inline values (scalars, simple
enums) and text, but **heap values** (struct / vector / struct-enum) are still
rejected. The reason is structural: a heap value is a `DbRef` into a store, and
`Stores::clone()` empties `allocations`, so a value built in the REPL's evaluation
store **cannot be aliased across** into the live (paused) store — the `DbRef` would
point at a record that does not exist there. Editing `pt = Point{x:9, y:9}` at a
breakpoint therefore needs the value **materialised in the live store**, and the
three ways to do that are each ugly on their own:

- run the constructor over the *suspended* stack (cursor/stack-clobber gymnastics);
- hand-roll a schema-guided deep-copy walker (per-type, fragile, re-implements
  construction);
- re-implement construction by writing records directly (the same, worse).

A **store change journal** dissolves the problem and is reusable far beyond it.

## The decision: an operation journal, not a post-hoc binary diff

A naïve "diff the bytes of two stores" tool is *dumb*: it rescans the whole store,
cannot tell an in-place edit from an insert/delete, and is fragile against
allocator metadata and `generation` bumps. Instead we **record changes at the
mutation site, through the normal store operations** — a list of records/vectors
changed, each tagged with its op and its physical `(store_nr, position, size)`.

Physical extent is the load-bearing detail: a *record number* alone is ambiguous
once slots are freed and reused (the allocator reuses positions), so an entry
carries `(position, size)` so it can be **placed, copied, or restored** without
re-deriving anything — which is precisely what makes inserts and deletes
replayable.

## The invariant

> **While recording is on, the records a change creates / frees / resizes are
> captured at the *structural* ops (`claim` / `delete` / `resize` — already
> special, already `generation`-bumped), and the bytes of a modified record are
> captured by a *whole-record snapshot* keyed on its `(store_nr, position, size)`
> — never by per-field interception. `addr_mut`, the hot field-write accessor, is
> never hooked. The resulting list is *sufficient* to replay the change forward
> onto a compatible store and to revert it backward.**

"Sufficient" is the testable part: forward-replay of an `Insert` list reproduces
the value (and its `DbRef`) in a target store; reverse-replay restores the target
to its pre-edit bytes.  Replay is **position-addressed** — it drives the allocator by
each entry's recorded position (`claim_at`), not best-fit `claim` — so the forward and
reverse passes are exact for *any* consumer (§ The model).

The invariant is deliberately phrased to keep the field-write path out of the
hook set — see § Hot-path discipline for *why* a whole-record snapshot suffices.

## Chokepoint — the mutation surface (confirmed)

| Op | Method | Hooked? | Cost |
|---|---|---|---|
| **Insert** a record | `Store::claim` (`store.rs:462`) | yes — already bumps `generation` (476) | cold (alloc path) |
| **Delete** a record | `Store::delete` (`store.rs:618`), via `free`/`free_named` | yes — already bumps `generation` (635) | cold |
| **Resize** (grow/shrink, may relocate) | `Store::resize` (`store.rs:583`) | yes — already bumps `generation` (586) | cold |
| **Modify** a field | `Store::addr_mut::<T>` (`store.rs:1376`) | **NO — never hooked** | **hot — must stay zero-cost** |

The three **structural** ops are single methods and **already bump a per-store
`generation` counter** (the coroutine `S28` mutation check). They sit on the
*allocation* path, which already does free-list work, so a `recording.is_some()`
branch + an append when recording is in the noise. These give Insert / Delete /
Resize for free.

`addr_mut::<T>` **is** the sole field-write funnel — every typed setter
(`set_int`, `set_u32_raw`, `set_boolean`) writes `*self.addr_mut(…) = …`, even the
raw `copy_nonoverlapping` writes take their `dst` from it, and a grep for raw
`store.ptr` writes outside `store.rs` finds only reads. **But it is also the
hottest path in the interpreter, so we deliberately do *not* hook it.** We don't
need to: see below.

## Hot-path discipline — why field writes need no hook

The naïve design marks a record dirty inside `addr_mut`, paying a branch on every
field write of every program forever. We avoid it entirely with one observation:
**the journal never needs to see a field write — it only needs the record's final
bytes, which it can read once at flush.** A modified record falls into exactly two
classes, and neither requires touching `addr_mut`:

- **A record the edit *creates*** — the constructor `claim`s it (Insert is
  journaled), then writes its fields. Those writes land *in the claimed record*, so
  a **whole-record snapshot at flush** (size is known from the `claim`) captures
  every one of them. The field writes are invisible to the journal and that is
  correct.
- **A pre-existing record the edit *modifies* in place** (`pt.x = 9`) — the
  debugger is the one doing the write, so it **marks that record explicitly**
  (`journal.touch(store_nr, pos, size)`) at its own edit site, snapshotting `before`
  then writing. One call, at the one place that knows what it touches — not a global
  hook.

So the hook set is the three **cold** structural ops plus **explicit marks at the
handful of debugger edit sites**. The hot `addr_mut` path keeps its current shape
byte-for-byte. Hot-path cost when recording is off: **zero** (no new code on the
field-write path); recording-on cost is paid only inside an edit, which is rare and
already interactive.

The completeness argument is the debugger's bounded scope: an edit only mutates
records it either creates (caught by `claim`) or explicitly edits (marked). There is
no third path — the debugger never silently mutates a record it did not construct or
name. (The future whole-execution journal — § Phasing 3 — *does* face arbitrary
program modifies; it must capture those the cheap way too: coarse, `generation`-gated
store-level dirty tracking + snapshot-the-changed-stores, never an `addr_mut` hook.)

## Scope — the debugger edit window

Recording is **off by default and costs nothing on the hot path** (the same shape
as `State.debug: Option<Debugger>` — one branch when absent). The debugger flips it
**on for the duration of one edit**, captures the changed-records list, flips it
**off**. So:

- no global `addr_mut` instrumentation on the execution hot path;
- only the records an *edit* touches are journaled;
- only **heap** stores matter — the suspended stack store is excluded (the live
  frame must survive the edit untouched).

This bounded scope is what keeps the first version small. Journaling *all* of
normal execution (for time-travel) is a future extension that additionally needs
the stack-frame raw writes funnelled — explicitly out of scope here.

## The three edit kinds, mapped

An entry is `{ store_nr, position, size, op: Insert | Free | Modify, before?, after? }`.
Recorded via the normal ops during the edit, one mechanism serves materialisation
**and** undo:

| Edit | Journaled | Materialise (apply) | Undo (revert) |
|---|---|---|---|
| `pt.x = 9` (scalar field) | one **Modify** of `pt`'s record (pos, range, old→new) | already in the live store | restore old bytes |
| `pt = Point{…}` (whole heap) | constructor's **Insert**s (pos, size, bytes) + a **Modify** of the frame slot's `DbRef` | **replay the Inserts into the live store** → the cross-store transfer | free the new records + restore old `DbRef` |
| `msg = "x"` (text) | `claim`/`set_str` **Insert** + the slot's `Str`/`String` **Modify** | replay + slot write | free + restore old slot |

The middle row is the payoff: **the journal is the cross-store transfer.** Run the
edit's constructor on a build store, journal its `Insert`s with their
`(position, size, bytes)`, and replay that list into the live store — generic, no
per-type walker, no constructor over the suspended stack. The `(position, size)`
is what lets replay place each record correctly even though slots get reused.

## The model — one position-addressed, reversible op log for every case

**Decision: a single model covers every case** — the bounded debugger edit, the
cross-store transfer, *and* the unbounded whole-execution journal (time-travel).
Where a bounded edit could get away with less (e.g. holding records claimed instead of
freeing them), the uniform model still applies; **the extra generality is the right
cost** — it is what lets *one* engine serve every consumer, and the unbounded journal
forbids the shortcuts the bounded one could take (it cannot hold a whole run's freed
records resident).

Every change is a **reversible entry** in one of three ops, each self-describing by
*physical position* and carrying the bytes its inverse needs:

| op | forward (apply / redo) | inverse (revert / undo) | payload |
|---|---|---|---|
| **Modify** `(rec, off, len)` | write `after` | write `before` | before + after |
| **Insert** `(pos, size)` | `claim_at(pos, size)` + write `after` | `free(pos)` | after |
| **Free** `(pos, size)` | `free(pos)` | `claim_at(pos, size)` + write `before` | before |

`apply` walks the log forward, `revert` walks it backward; each entry's inverse is
**self-contained** — it depends only on its own recorded bytes and position, so
reverse-replay needs no global reconstruction.  The same engine materialises a value
into the live store (cross-store transfer), redoes, and undoes — bounded edit or whole
run, no second mechanism.

**The keystone — `claim_at(pos, size)`.**  Re-materialising a record at an *exact*
position is what best-fit `claim` will not do (it picks the smallest fitting block).
`claim_at` carves `[pos, pos+size)` out of the free space covering it: find the free
block containing `pos`, **absorb consecutive adjacent free blocks** until they span the
region, then re-head it as `[start, pos)` free | `[pos, pos+size)` claimed |
`[pos+size, end)` free (a 3-way split when coalescing left `pos` mid-block).  The
absorb step is load-bearing, not cosmetic: `delete` coalesces only *forward*, so a freed
predecessor stays a **separate** free block until lazy `coalesce_free` runs — the region
can be entirely free yet *fragmented* across several blocks (probe #6c caught a
single-block version failing exactly here).  At revert time the region is *guaranteed
free*: every claim made after the original `free(pos)` is a later log entry, already
reverted (freed) before we reach this one — so `claim_at` can always carve it.

This **subsumes probe #2 and is strictly more robust.**  Probe #2 asked whether a
cloned store's best-fit `claim` would *coincidentally* reproduce positions (so an
`Insert` lands where its internal `DbRef`s point).  `claim_at` removes the coincidence:
positions are **recorded and forced**, so replay no longer depends on the allocator
staying deterministic — `fl_take_ge` could change tomorrow and replay would be
unaffected.  Probe #2's determinism becomes a *nice-to-have*, not load-bearing.

### The relocating grow, in this model

`v.push(x)` against a live vector, when `Store::resize` (`store.rs:583`) cannot extend
in place, relocates: `claim(new)` + `copy(old→new)` + `delete(old)`, then `vector.rs`
writes the new record number into the owning `u32` cell (`vector_append:166`,
`insert_vector:53`, `sorted_new:209`).  A vector is a two-level handle — an owning cell
at `(container_rec, pos)` holds `vec_rec`; the record is
`[ claim:i32 @0 | len:u32 @4 | elems @8 + i·size ]`.

It journals as three ordinary entries:

- **Insert(new, after-bytes)** — the grown record (the `claim` structural-op hook).
- **Modify(owning cell: old→new)** — the pointer-flip.  It goes through
  `set_u32_raw`→`addr_mut`, the **unhooked** hot path (probe 5b), so it is marked
  explicitly at the resize-caller sites (a closed set: `vector_append` / `insert_vector`
  / `sorted_new` / `structures.rs`) — *observable resize*: while recording, the
  relocation reports `(old, new)` so the caller marks the 4-byte cell.
- **Free(old, before-bytes)** — the freed record, snapshotted to the blob.  Its
  `before` is captured **before `delete` runs**, because a freed block's body is
  repurposed *instantly* as a free-tree node (`FL_LEFT`@4 / `FL_RIGHT`@8, over the
  vector's `len` + element 0 — probe 5a).  The live store then reclaims the space.

So undo re-materialises `old` (`claim_at(old.pos)` + restore its blob bytes) and flips
the cell back; redo re-materialises `new` and flips forward (probe 5c: the pointer-flip
*is* the switch once both records exist).  **No record is held claimed for the
session** — the live store stays compact even across an unbounded run, and the history
lives in the file-backed blob where it belongs.  The earlier "stay-claimed" sketch
was rejected for exactly this reason: it crams the history into the live store, which
the unbounded journal cannot afford (see § Open design points).

`v[i] = x` (no relocation) is a lone **Modify** of `(vec_rec, 8 + i·size)`; an in-place
grow (resize extends, same `vec_rec`) is a **Modify** of the `len` field (its undo
restores the shorter length — the trailing slack is harmless, the vector never reads
past `len`).  Same three ops, no special cases.

## Storage model — an index store + a blob file (two artifacts)

The journal is **two artifacts**, split by what each part is good at — structure
where it pays, raw bytes where it does not:

- **Blob — a plain file, always.** An append-only byte stream holding the
  variable-length payload (the `before`/`after` bytes of each change).  No headers,
  no allocator: appending is a bump, and a WAL never frees mid-stream, so a store's
  record/free-list machinery would be pure overhead.  It is *always* a file
  (`append` to write, seek + `read` to replay) — a store would force a needless
  RAM-or-mmap dual-mode on what is fundamentally a byte stream, and "a file" is the
  VirtFS in the browser, so the in-RAM / in-browser case still works.

- **Index — a store holding one growing fixed-stride array.** One **entry per
  change**, fixed width, appended to a single array that is the store's *only*
  occupant.  Because nothing sits after it, growth just extends the store's tail:
  the array never relocates within the store, element offsets stay valid, and
  append is O(1).  No secondary index (that would promote it to a keyed collection)
  — a plain vector, walked forward to `apply`, backward to `revert`, random-access
  by element.

**The entry (fixed 24 bytes, little-endian so it is on-disk portable):**

| field | type | meaning |
|---|---|---|
| `op` | u8 | `Modify` (built) / `Insert` / `Free` |
| `store_nr` | u16 | target store (`Stores::allocations` index) |
| `rec` | u32 | target record (for `Insert`/`Free`, the record's position) |
| `off` | u32 | byte offset of the changed region within the record (0 for `Insert`/`Free`) |
| `len` | u32 | region / record width |
| `blob_at` | u64 | offset in the blob of the payload |

The payload depends on the op: a **Modify** carries `before` then `after` (each `len`
wide; `before` at `blob_at`, `after` at `blob_at + len`); an **Insert** carries only
`after` (the new record's bytes, for forward materialisation); a **Free** carries only
`before` (the freed record's bytes, snapshotted *before* `delete` — probe 5a — for
reverse re-materialisation via `claim_at`).

**mmap is optional, and free on the store side.** The index *is* a `Store`, so it
inherits the store's RAM-or-mmap duality with no extra code — a throwaway debug
session keeps it in RAM; a persisted one mmaps it.  Combined with the data stores
(and the loft2 schema store), mmap'ing all of them is the AS/400 single-level store:
persistent-by-default, no memory/disk seam.  The blob carries persistence on its own
(it is a file either way).

**The commit rule (load-bearing once it is two files): the index entry is the
commit point.** Append the payload to the blob *first*, write the index entry
*last*.  On recovery, trust the index up to its last complete element and ignore any
half-written blob tail beyond it — the one ordering rule that separates a
recoverable WAL from a corrupt one.

**Built so far:** the `Modify` slice (in-place field/element edits, correct by
construction — § Hot-path discipline) **and the keystone `Store::claim_at(pos, size)`**
(`src/store.rs`) — the exact-position carve that unlocks `Insert` + `Free` and makes
forward/reverse replay position-exact for every consumer.  Probe #6 confirms it
(position-exact + bidirectional round-trip across a coalescing-heavy sequence).  Since
`claim_at` *forces* positions, probe #2's `claim`-determinism is no longer load-bearing.
Still to build: the `Insert`/`Free` entry path + the resize-caller pointer marks, wired
into the debugger edit (§ Phasing 3).

## Falsification probes (run before building)

The invariant rests on claims that must be *probed*, not assumed:

1. **Capture completeness — CONFIRMED for the edit scope.** Structural changes
   funnel through `claim`/`delete`/`resize` (the hooked set); in-place modifies are
   only ever made by the debugger, which marks them explicitly — so an edit's
   record set is fully captured *without* hooking `addr_mut`. Probe: grep confirms
   `addr_mut` is the sole external write accessor (so nothing creates a record off
   the `claim` path), and the debugger edit sites are a closed, countable set.
2. **Replay-position determinism — CONFIRMED.** Forward-replay of an `Insert`
   reproduces the value at a `DbRef` valid in the target *iff* the target store's
   allocator places the record at the same position the build store did.  `claim`
   is a **pure deterministic function of allocator state**: the same claim/free/
   coalesce/grow sequence on two independent stores hands out identical positions
   (`tests::claim_is_deterministic_from_history`), and since Rust's `HashSet` is
   randomly seeded per construction, a match proves the `claims` set never leaks
   into position selection.  So a store cloned from the live store, run through the
   constructor, claims at positions that are *also* free in the live store — replay
   is `live.claim(size)` (returns the recorded position) + `write_span`, with **no
   `DbRef` remap**.  This unlocks the `Insert`/`Free` (whole-value) slice.
3. **Stack/heap separation (CONFIRMED-ish).** Struct/vector construction claims
   into heap stores (`OpDatabase` → a fresh store via `claim`), distinct from the
   stack store (`stack_cur.store_nr`). Probe: a struct edit must leave
   `stack_cur`'s store bytes byte-identical except the edited slot.
4. **Revert fidelity.** After reverting a whole-heap edit, the paused frame +
   store are byte-identical to pre-edit. Probe: snapshot → edit → revert → diff.
5. **The relocating-grow structural half — PROBED (three tests in
   `src/database/journal.rs`).**  The model's relocation entry rests on three facts,
   each with a guard test:
   - **5a — a freed block's body is immediately repurposed** (`freelist_repurposes_a_freed_blocks_body`).
     `delete` → `fl_insert` writes the free-tree links at offsets 4/8, over the vector's
     `len` + element 0.  **This falsified the first draft** ("a freed record's bytes
     survive for undo") and pins the ordering rule: a `Free` entry must snapshot
     `before` **before `delete` runs**, never after.  (If a future allocator stopped
     embedding nodes in freed blocks this test would flip — and a cheaper free-and-read
     would become available.)
   - **5b — a relocating grow flips the owning u32 cell through the unhooked path**
     (`relocation_flips_the_owning_pointer`).  Confirms the structural-op hooks alone
     miss the pointer-flip, so it needs an explicit Modify at the resize-caller sites.
   - **5c — once both records exist, the owning cell alone selects which the value sees**
     (`deferred_free_pointer_flip_round_trips`).  The pointer-flip *is* the switch:
     apply re-materialises `new` then flips forward; undo re-materialises `old`
     (`claim_at` + blob restore) then flips back.
6. **`claim_at` position-exactness + bidirectional round-trip — CONFIRMED, after the
   hardening pass caught a real bug (three tests in `src/database/journal.rs`).**  The
   whole single model rests on `claim_at(pos, size)` reproducing a record at its *exact*
   recorded position — so `Insert`-apply and `Free`-revert are position-exact.
   - **6a — the mid-block carve** (`claim_at_carves_a_mid_block_region`).  Frees three
     adjacent records so one slot is *strictly mid-block* in the coalesced free block,
     then re-claims it: `claim_at` does the full 3-way split and the chain still tiles.
   - **6b — bidirectional position-exact replay** (`bidirectional_position_exact_replay`).
     A coalescing-heavy edit logged as `Insert`/`Free`; **revert** restores the baseline
     byte-for-byte and **re-apply** restores the edit.
   - **6c — fuzzed bidirectional replay** (`bidirectional_replay_fuzz`, 12 seeds ×
     600 ops).  **This is where the predicted coalesce-corner falsification actually
     landed.**  6a/6b held only because their frees happened to forward-coalesce; the
     random order produced a freed *predecessor* left as a **separate** free block (lazy
     `coalesce_free` not yet run), so the region was free but *fragmented* — and the
     first `claim_at` (single covering block only) failed "overruns its free block."
     **Root:** `claim_at` mirrored `claim`'s position but not its `coalesce_free`.
     **Fix:** `claim_at` now absorbs consecutive adjacent free blocks across the region.
     Re-ran clean.  (It surfaced in the forward / re-apply pass; revert was already
     robust by the reverse-order invariant below — but the same fix hardens both.)
   - **The reverse-order invariant (why revert can't falsify):** in reverse order, *any*
     claim that ever touched a freed region is a later log entry, reverted-to-free
     **before** that region's `claim_at` runs — so `[pos, pos+size)` is always free when
     `Free`-revert re-claims it (free as a *region*; `claim_at` now spans fragments).

## Open design points

- **Granularity — decided: record-level, captured by snapshot.** A whole-record
  snapshot (size known) at flush, never per-field interception — this is what keeps
  the hot `addr_mut` path unhooked (§ Hot-path discipline).
- **Reversibility — capture before+after.** Pure replay-into-live-store needs only
  `after`; undo and "show the delta" need `before` (snapshot-on-first-touch).
  Record both — it is the superset and undo is the natural live-edit companion.
- **Freed records — decided: snapshot to the blob + `free`, *not* stay-claimed.**  An
  earlier sketch kept the old record *claimed* for the session (undo = pointer-flip, no
  snapshot).  Rejected: it holds live-store space for the whole edit, which the
  **unbounded whole-execution journal cannot afford** (a long run frees unboundedly
  many records).  The single model instead snapshots a freed record's `before` to the
  blob — taken *before* `delete` runs, since the body is repurposed instantly (probe
  5a) — and frees it, so the live store stays compact and the history lives in the
  file-backed blob.  Undo re-materialises via `claim_at(pos)` + the blob bytes.  The
  cost (an O(record) blob write per free, vs zero for stay-claimed) is the right price
  for one model that also serves the unbounded case.
- **Where the journal lives.** A `recording: Option<Journal>` on `Stores`
  (off = `None`, one branch), entries global and ordered by `(store_nr, position)`
  so replay/serialise is a single ordered pass.

## Consumers (why it is worth more than one feature)

The first consumer is **live edits**; the same primitive is load-bearing for the
lavition direction (each is "see/transfer what changed in the store"):

- **Full live edits + undo** — this doc.
- **Incremental serialisation** (loft2 store-resident IR → mmap/JSON): write only
  changed records, so continuous state-saving stays cheap.
- **Time-travel debugging** — a per-step journal → step *backward* (needs the
  whole-execution extension above).
- **State continuity / hot-reload** — reconcile state across a code swap.
- Later: network/multiplayer sync (game-client) is the same delta-shipping shape.

## Phasing

1. **`Modify` slice — DONE** (`src/database/journal.rs`).  `recording: Option<Journal>`
   shape, record + apply + revert for in-place region edits, the two-artifact storage
   model.  Covers field / element edits (`pt.x = 9`, `v[i] = 5`) by construction — a
   single `Modify`, no materialisation, the cheapest heap edit.  The hot `addr_mut` path
   is untouched.
2. **`claim_at` keystone + the `Insert`/`Free` entry path — DONE** (`src/store.rs`,
   `src/database/journal.rs`).  The exact-position 3-way-split carve (§ The model), plus
   `Journal::record_insert` (apply = `claim_at` + write `after`; revert = `free`) and
   `record_free` (apply = `free`; revert = `claim_at` + write `before`, snapshot taken
   pre-`delete`).  `apply`/`revert` dispatch on the op.  Tested by
   `insert_and_free_round_trip` and `relocation_insert_modify_free_round_trips` (the
   full `{ Insert + Modify + Free }` relocation shape, both directions).
3. **Debugger heap edits — wire it in (in progress).**  Remaining: `recording` on the
   stores (a per-`Store` change buffer that `Stores` drains into the unified `Journal` —
   `Store` methods don't know their own `store_nr`, so a back-reference is out; one
   branch on the *cold* `claim`/`delete`/`resize` paths when off); the explicit
   pointer-flip marks at the resize-caller sites (`vector_append` / `insert_vector` /
   `sorted_new` / `structures.rs`); build the edit value in a *live-cloned* store with
   recording on so its `Insert` positions are free in the live store (probe #2 / the
   `claim_at` no-remap property); **replay into the live store** + write the frame slot
   to finish the heap edit rejected at `set_frame_literal` (`state/mod.rs`); revert for
   undo.  Probes 1, 3, 4 graduate to the debugger regression suite.
4. **Whole-execution journal** — funnel the stack-frame raw writes, journal all of
   execution → time-travel + incremental serialisation.  The same model, unbounded;
   the snapshot-to-blob freed-record handling (§ Open design points) is what lets it
   run without holding the run's freed records resident.

## See also

- [README.md](README.md) — the @PLN15 sub-arc table (G1 / heap live edits).
- [../future/38-loft-store-durable/README.md](../future/38-loft-store-durable/README.md)
  — @PLAN38 durable stores; its **Tier 3 (WAL)** builds on this journal (§ Convergence
  there) — the persistence/mmap consumer of this substrate.
- [../../DATABASE.md](../../DATABASE.md) — store allocator, `Stores`, `DbRef`.
- [../../GOALS.md](../../GOALS.md) Goal E — robustness by subtraction (one owned
  home for the shared medium — here, the journal owns "what changed").
- The **`design-protocol` skill** (via the [DESIGN_PROTOCOL.md](../../DESIGN_PROTOCOL.md)
  stub) — the design-as-hypothesis protocol this doc follows.  Freed-record handling is
  a live instance: a clean first draft ("freed bytes survive for undo") that probe 5a
  **falsified**, an interim *stay-claimed* fix, and — once the unbounded whole-execution
  consumer ruled out holding records resident — the settled *snapshot-to-blob +
  `claim_at`* model (§ The model, § Open design points).  The keystone was then
  **predicted to falsify** at the coalesce corner.  The hand-written probes (6a/6b)
  *held* — but only because their frees happened to forward-coalesce, so they were too
  weak to reach the predicted corner.  The **fuzz hardening (6c) hit it**: a free-but-
  *fragmented* region `claim_at` couldn't carve.  Fixed, re-run clean.  Textbook
  protocol: the prediction was right, the first probes were too weak to confirm it, and
  widening the matrix is what turned the prediction into a caught bug.
