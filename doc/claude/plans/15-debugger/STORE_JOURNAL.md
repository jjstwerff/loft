<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 15.J — Store change journal (the live-edit substrate)

> **Identity:** a design sub-doc of `@PLN15` (debugger). Slug `store-journal`.
> **Status:** design (`Modify` slice built — `src/database/journal.rs`). Load-bearing
> claims **confirmed** — the mutation chokepoint (§ Chokepoint), replay determinism
> (probe #2), and the relocating-grow structural half (§ The fourth edit kind; probes
> #5a–c, where 5a falsified the first draft and reshaped *deferred-free* into
> *stay-claimed*).

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
to its pre-edit bytes.

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

## The fourth edit kind — in-place heap growth (a relocating vector resize)

The three kinds above cover scalar / whole-value / text.  A fourth is an **in-place
grow of an existing heap collection** — `v.push(x)`, `v.insert(…)`, a keyed `sorted`
insert — evaluated at the breakpoint against a *live* vector.  Two sub-cases, split by
whether the backing record stays put.

A vector is a **two-level handle**: an owning `u32` cell at `(container_rec, pos)`
holds the backing record number `vec_rec`; that record is
`[ claim:i32 @0 | len:u32 @4 | elems @8 + i·size ]`.

- **Element overwrite `v[i] = x` — a clean Modify.**  The element lives in the
  existing backing record, so the edit writes `(vec_rec, 8 + i·size)` — one **Modify**,
  no structural op, no pointer-flip.  The cheapest heap edit (§ Phasing 2).

- **A grow that relocates — a pointer-flip over two conserved records.**  When
  `Store::resize` (`store.rs:583`) cannot extend in place it **relocates**:
  `claim(new)` + `copy(old→new)` (the data is *conserved* — copied from offset 4 on) +
  `delete(old)`, and `vector.rs` then writes the new record number into the owning cell
  (`vector_append:166`, `insert_vector:53`, `sorted_new:209`).

The naïve journal snapshots the whole grown vector's bytes (O(N) per grow).  The
structural half records the grow as **{ Insert(new) + Modify(owning cell: old→new) +
deferred Free(old) }** — O(1), because the data is conserved in the two records the
edit session keeps alive, so undo is a pointer *write*, not a byte restore.  Two facts,
**both probed (§ Falsification probes 5a/5b)**, make it correct:

1. **Deferred free means *stay-claimed*, never "free but remember the bytes."**  The
   allocator embeds its red-black free-tree node links *inside* freed blocks (`FL_LEFT`
   @ offset 4, `FL_RIGHT` @ offset 8 — exactly the vector's `len` and element 0), so
   `delete(old)` overwrites old's data **the instant it runs**; there is no window in
   which a freed record's contents survive (probe 5a — this falsified the first draft,
   which assumed they did).  So a **recording-mode resize keeps the old record
   claimed**: it allocates + copies + flips the pointer but does **not** `delete(old)`,
   recording old in a pending-free set instead.  The physical free happens only at a
   session boundary — commit frees the olds, discard / undo frees the news.  And
   **recording mode forces relocation** (resize never extends in place while recording)
   so every grow has this one shape, *subtracting* the in-place-extend case — whose
   undo would otherwise need free-tree surgery (restore the header + re-insert the
   absorbed block) — entirely (Goal E).

2. **The pointer-flip needs an explicit Modify.**  The owning-cell write goes through
   `set_u32_raw` → `addr_mut`, the **unhooked** hot path, so the three structural-op
   hooks never see it (probe 5b).  It is captured by an explicit mark at the
   resize-caller sites — a closed, countable set (`vector_append` / `insert_vector` /
   `sorted_new` / `structures.rs`) — the same explicit-mark mechanism the in-place
   modifies use (§ Hot-path discipline), extended to "the pointer-flip after a
   recording-mode relocation."  This is **observable resize**: while recording, the
   relocation reports `(old, new)` so the caller can mark the 4-byte cell.

The payoff (probe 5c): with both records kept claimed, **apply / redo flips the owning
cell to `new`, revert / undo flips it back to `old` — no data snapshot either way.**
The blob payload for the relocation is the 4-byte old/new record numbers, not the
vector.  `Insert(new)` carries `new`'s bytes only for the *cross-store* transfer
(build-store → live-store replay); a same-store live grow's undo never reads them.  The
event vocabulary stays exactly `{ Modify, Insert, Free }` — "deferred" is *when* the
Free physically executes (session boundary), not a new op.

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
| `op` | u8 | `Modify` (later `Insert` / `Free`) |
| `store_nr` | u16 | target store (`Stores::allocations` index) |
| `rec` | u32 | target record |
| `off` | u32 | byte offset of the changed region within the record |
| `len` | u32 | region width; `before` and `after` are each `len` bytes |
| `blob_at` | u64 | offset in the blob: `before` at `blob_at`, `after` at `blob_at + len` |

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

**First implementation:** `Modify` only (the in-place field/element edits, correct by
construction — § Hot-path discipline).  `Insert` / `Free` replay (whole-value) lands
once probe #2 graduates — which it now has (`claim` is deterministic, so replay needs
no `DbRef` remap).

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
   `src/database/journal.rs`).**  The "fourth edit kind" above rests on three claims,
   each with a guard test:
   - **5a — a freed block's body is immediately repurposed** (`freelist_repurposes_a_freed_blocks_body`).
     `delete` → `fl_insert` writes the free-tree links at offsets 4/8, over the
     vector's `len` + element 0.  **This falsified the clean first draft** ("a freed
     record's bytes survive for undo") and forced *deferred-free = stay-claimed*.  If a
     future allocator stopped embedding nodes in freed blocks this test would flip — and
     the simpler "free + remember" design would become available again.
   - **5b — a relocating grow flips the owning u32 cell through the unhooked path**
     (`relocation_flips_the_owning_pointer`).  Confirms the structural-op hooks alone
     miss the pointer-flip, so it needs an explicit Modify at the resize-caller sites.
   - **5c — stay-claimed makes the grow a pure pointer-flip over two conserved records**
     (`deferred_free_pointer_flip_round_trips`).  Confirms apply/revert need no data
     snapshot — the owning cell alone selects pre-grow vs grown, both records kept live.

## Open design points

- **Granularity — decided: record-level, captured by snapshot.** A whole-record
  snapshot (size known) at flush, never per-field interception — this is what keeps
  the hot `addr_mut` path unhooked (§ Hot-path discipline).
- **Reversibility — capture before+after.** Pure replay-into-live-store needs only
  `after`; undo and "show the delta" need `before` (snapshot-on-first-touch).
  Record both — it is the superset and undo is the natural live-edit companion.
- **Don't eagerly free on whole-value replace or a relocating grow — decided:
  stay-claimed.** Keep the old record *claimed* for the whole session so undo can flip
  back to it; free only at session boundary (commit frees olds, discard frees news).
  Not merely "so undo can restore it" — a freed block's body is *instantly* repurposed
  as a free-tree node (probe 5a), so there is no "freed but still readable" state to
  rely on. Deferred free **is** stay-claimed.
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

1. **MVP — record + apply + revert, debugger-scoped.** `recording: Option<Journal>`
   on `Stores` (off = `None`, one branch on the *cold* `claim`/`delete`/`resize`
   paths only); explicit `journal.touch(...)` at the debugger edit sites; capture
   during `debug_set`; **replay Inserts into the live store** to finish heap live
   edits; revert for undo. The hot `addr_mut` path is untouched. Probes 1–4
   graduate to the debugger regression suite.
2. **Field/element edits** (`pt.x = 9`, `v[i] = 5`) — a single `Modify`, no
   materialisation; the cheapest heap edit, lands alongside the MVP.
3. **Whole-execution journal** — funnel the stack-frame raw writes, journal all of
   execution → time-travel + incremental serialisation. Its own slice.

## See also

- [README.md](README.md) — the @PLN15 sub-arc table (G1 / heap live edits).
- [../future/38-loft-store-durable/README.md](../future/38-loft-store-durable/README.md)
  — @PLAN38 durable stores; its **Tier 3 (WAL)** builds on this journal (§ Convergence
  there) — the persistence/mmap consumer of this substrate.
- [../../DATABASE.md](../../DATABASE.md) — store allocator, `Stores`, `DbRef`.
- [../../GOALS.md](../../GOALS.md) Goal E — robustness by subtraction (one owned
  home for the shared medium — here, the journal owns "what changed").
- The **`design-protocol` skill** (via the [DESIGN_PROTOCOL.md](../../DESIGN_PROTOCOL.md)
  stub) — the design-as-hypothesis protocol this doc follows.  § The fourth edit kind is
  a live instance: a clean first draft ("freed bytes survive for undo"), a probe that
  **falsified** it (5a), and the sharper invariant that replaced it (*deferred-free =
  stay-claimed*).
