<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Design — iterate an exposed (read-only-pinned) keyed collection

A slice of [@PLN105](105-ffi-deliver-layout-bridge.md) (the FFI deliver / layout
bridge). Surfaced by the `routing` consumer (`docs/loft-feedback.md`, 2026-07-22,
"`expose` UN-RETRACTED"). **Status: Steps 0-4 DONE — an exposed hash/radix now iterates
(new `on=4` mode), correct and leak-clean for complete / break / repeated / nested walks
on both backends (loop-epilogue conditional free). Residuals: a `return` out of an exposed
loop leaks the scratch (caught by the leak check), and bounded-range over an exposed
non-hash source still errors (Step 5). Step 3's header approach was falsified for `on=3`
en route but is sound under the compile-time `on=4` split.** This doc is the single source
of truth; it leads
with the probes/tooling/oracles because the risk is entirely in the store-lifetime
subsystem (loft weakness #1) and the build must be gated behind instruments that can
*falsify* each step — which is exactly what caught Step 3.

---

## The observed failure (reproduced, both backends)

```loft
struct Tile { tkey: integer, ox: integer }
fn main() {
  h: hash<Tile[tkey]> = [];
  h += [Tile{tkey:1, ox:10}]; h += [Tile{tkey:2, ox:20}];
  for t in h { }           // fine
  expose(1, h);            // pins h's store read-only (lock_store)
  for t in h { }           // PANIC — src/store.rs:647 "Claim on read-only store (size=2)"
}
```

In wasm the panic is a **silent trap** — the kernel dies mid-command, never emits its
terminator, the page hangs. Reads/lookups after `expose` are fine; only **iteration**
fails.

## Root cause (grounded)

Ordered iteration of a keyed collection (`hash`/`index`/`radix`/`spatial`) materialises
a sorted **rec-nr scratch vector** and walks it (the `Ordered`, `on=3` path). The
scratch is claimed **inside the collection's own store**:

- `Stores::build_rec_scratch` (`src/database/allocation.rs:1106`) does
  `self.claim(hash_ref, …)` — a claim in `hash_ref.store_nr`. Comment "C60 piece 3 edit
  A" says this is deliberate: co-locating lets the yield reuse one `store_nr`.
- `expose` → `Stores::lock_store` sets `Store::read_only = true`; `Store::claim`
  (`src/store.rs:641-651`) hard-asserts `!read_only`. → panic.
- The yield reads the scratch and constructs the element from **`data.store_nr`** (the
  scratch's store) in **two mirrored sites**: `src/state/io.rs:1188-1203` (interpreter
  `step`) and `src/codegen_runtime.rs:974-991` (native runtime `step`). Both do
  `DbRef{ store_nr: data.store_nr, rec, pos: 8 }`.

Why `read_only` is a hard wall and not just a flag to relax: `Store` is a raw
`*mut u8` (`src/store.rs:137`) grown by `resize_store` (realloc → **the base moves**).
The pin's real job is *the exposed base must not move*, because JS/wasm holds that base
+ a descriptor across frames. A scratch claim that grows the store would move it → a
UAF in the browser reader. Writes are only `debug_assert`-guarded (`addr_mut`,
`src/store.rs:1747`), so the wall is specifically `claim` (grow) — which is correct.

## The exact-invariant framing (why capture-and-diff, not "explore")

This is a store-lifetime problem: the answer is a **construction to recover**, not a
space to search. The proven sibling **already runs** — iterating a *writable* hash
yields the right thing. So the method is: **capture the writable-source artifact and the
exposed-source artifact into files and diff.** The residual divergence *is* the
construction. Plot the target end-state for the exposed case:

| after `for t in exposed_h { }` | target |
|---|---|
| exposed store base pointer | **unchanged** (JS's handle stays valid) |
| exposed store bytes | **unchanged** (no claim/write/free in it) |
| yielded element sequence | **identical** to the writable-source golden: same `store_nr` (= the hash store), same recs, same key order |
| scratch | built + read + freed in a **different** store |
| store/record census after | back to **baseline** (no scratch leak) |

## The invariant (the hypothesis to test)

> **The scratch's store and the records' store are decoupled: the scratch carries the
> source (records) `store_nr` explicitly; iteration reads the scratch wherever it lives
> and yields records into the *source* store. When the source is read-only, the scratch
> lives in a distinct writable store.**

The fast path (writable source) keeps the scratch co-located — then *source `store_nr`
== scratch `store_nr`*, so the same rule yields byte-identical behaviour with the
decoupling machinery merely dormant.

### Failure paths (written down first — this is where the invariant earns its scope)

1. **Wrong yield store.** If the yield keeps using `data.store_nr` (scratch store) while
   the scratch moved out, elements resolve into the *scratch* store → garbage / UAF.
   *(Guarded by: yield reads the header's source `store_nr`.)*
2. **Scratch leak.** The scratch var is typed `Reference(content, hash_deps)`
   (`src/parser/collections.rs:1690`) — it carries the hash's dep list, so free-cleanup
   treats it as a **borrow** and never frees it independently. Correct while co-located
   (freed with the hash store); **wrong** once the scratch lives elsewhere — its records
   are never reclaimed → the dedicated store grows every iteration. **This is the hard
   part.** *(Guarded by: the leak oracle; see Open question A.)*
3. **Bounded range over an exposed source.** `iterate()` `on=3` for a *range*
   (`xs[(x,y)..]`, sorted/index/radix slices) calls `vector::ordered_find`, which reads
   **record keys** via `data.store_nr`. Decoupling makes that the scratch store → key
   reads hit the wrong store. A **hash never hits this** (its iteration is always full →
   the empty-bounds sentinel path, no `ordered_find`), so routing's case is clean; other
   keyed collections range-iterated *while exposed* are not. *(Guarded by: matrix cell +
   Step 4 — handle or reject-cleanly, never silently wrong.)*
4. **Nested / concurrent iteration** of exposed collections needs >1 live scratch at
   once. *(Guarded by: the dedicated store holds N scratches as ordinary records, not a
   single reset buffer — see Open question A.)*
5. **`par` worker** iterating an exposed hash: the `WorkerStores` clone must carry the
   dedicated-store handle or lazily make its own. *(Guarded by: matrix cell; low
   priority.)*

## Re-assertion sites — the prospective brittleness count (N)

The source `store_nr` must be threaded at each site below; **omission is silent** (wrong
`store_nr` = UAF, not a compile error), so `N × silence` is the brittleness known *now*:

| site | file | role |
|---|---|---|
| write source `store_nr` into header | `allocation.rs` `build_rec_scratch` | 1 |
| read it at yield (interpreter) | `io.rs:1188` `step` `on=3` | 2 |
| read it at yield (native) | `codegen_runtime.rs:974` `step` `on=3` | 3 |
| read it for range key-compare | `vector::ordered_find` (bounded only) | 4 |

**N = 3 (full) / 4 (with range).** The two yield sites (2, 3) are *already* a
copy-paste duplication of the same `on=3` arm. **Cure: collapse them into one shared
`step_ordered()` helper first** (Step 1) — a behaviour-preserving refactor that drops N
from 3→2 and makes the fix a single-site change. This is step 2 of the protocol paying
off before any behaviour changes.

---

## PROBES — the boundary matrix (build FIRST, `/tmp` on `--interpret`, then graduate)

Every cell gets a **hand-computed** expected value; agreement between two runs is not a
pass. Assert **value AND order AND leak** in each. The load-bearing cell is
`exposed-hash-full` vs `writable-hash-full` **agreement** (the capture-and-diff target).

| axis | cells |
|---|---|
| collection | `hash` · `sorted` · `index` · `radix` · `spatial` |
| source state | **writable** (proven sibling) · **read-only** (`expose`d) |
| extent | **full** (`for e in c`) · **bounded range** (`c[a..b]`, non-hash) |
| body | empty · reads a field (`t.ox`) · builds text (`"{t.tkey}"`) · appends after `release` |
| backend | `--interpret` · `--native` (values identical) |
| census | stores/records freed to baseline after the loop |

Seed probe (writable + exposed, hand-computed `123` / sum `6` / post-release sum `10`)
is already written — `$CLAUDE_JOB_DIR/tmp/mtx.loft`. The real store lives at
`routing/_site/stores/enschede.layout.store` (1089 tiles) — the scale cell.

**Prove the harness can fail:** a no-output cell is vacuous; force one failing cell
(e.g. assert `124`) and see it red before trusting the greens.

## TOOLING — the instruments each step leans on

| instrument | state | use |
|---|---|---|
| `expose_iter_probe.loft` | **exists** (routing `tools/`) — `read`/`iter`/`release` ops on a real store | graduate to `tests/scripts/pln105-expose-iterate.loft` |
| `LOFT_ITERATE_TRACE` | **exists** (`io.rs:960`) — start/finish/yield per step | confirm the yield's `store_nr` per element |
| store census (`Stores::peak`, "N stores not freed at exit") | **exists** | the **leak oracle** backbone — assert baseline after the loop |
| **exposed-store fingerprint** | **NEW, ~15 lines** | capture the pinned store's base ptr + a byte-hash before/after the loop; the *invariant* oracle (base+bytes unchanged) |
| **capture-and-diff harness** | **NEW, tiny** | dump `(store_nr, rec, key)` per yielded element for writable vs exposed into two files; `diff` must be empty |
| `LOFT_STORES=log` | **exists** | store watermark trace when a leak cell goes red |

## ORACLES — what proves each step (not "it ran")

- **Value oracle** (capture-and-diff): exposed-source yield sequence `==` writable-source
  golden, both backends. The golden is the *proven sibling's* captured artifact.
- **Invariant oracle**: the exposed store's base pointer **and** byte-hash are unchanged
  across the loop. This tests the thing JS actually depends on — the buffer did not move
  and its bytes did not change. **This is the fix's real correctness gate.**
- **Leak oracle**: store + record census returns to baseline after the loop (the
  dedicated scratch's records are freed).
- **Refactor oracle** (Step 1 only): emitted IR + native Rust **byte-identical**
  before/after, and every existing iteration test value unchanged, both backends
  (`loft introspect` diff — the loft-codegen gate for a behaviour-preserving change).

---

## Safe small steps (each lands green + committed before the next)

**Step 0 — instruments, no product change.** Write the matrix (`/tmp`, `--interpret`),
hand-compute every cell, prove the harness can fail. Build the invariant oracle
(fingerprint) and the capture-and-diff harness. Capture the **writable-source golden**.
Graduate `expose_iter_probe` to `tests/scripts/`. *Oracle: goldens captured; a forced-
wrong cell is red.*

**Step 1 — collapse the two yield mirrors (behaviour-preserving refactor).** Extract the
`on=3` arm shared by `io.rs:1188` and `codegen_runtime.rs:974` into one
`step_ordered(data, cur, all) -> DbRef`. Drops N 3→2. *Oracle: byte-identical IR + native
Rust + all iteration values unchanged, both backends (refactor oracle).* Ship-safe alone.

**Step 2 — safety net: clean error, not a silent hang (ship-first, independent).** In
`build_rec_scratch`, if the target store is `read_only`, `raise_runtime` a clear kind
(*"cannot iterate an exposed collection — call `release()` first, then re-`expose()`"*)
instead of falling into the panicking `claim`. Converts the wasm silent-trap/hang into an
actionable halt on **every** backend. This is the routing-stated fallback and de-risks
everything after it (no more silent hang while the real fix is built). *Oracle: exposed
iteration → clean typed error, not panic/hang, both backends; matrix's writable cells
unchanged.*

**~~Step 3 — thread the source `store_nr` through the header (additive, inert).~~
FALSIFIED — this is the design-protocol earning its keep.** The plan was a 2-word scratch
header carrying the source `store_nr` at offset 8, read back by `step_ordered` for the
yield. Implemented, it broke `502-keyed-slice-for-only` and
`85-store-lifetime-claims-keystone` on **both** backends (a bad `store_nr` → out-of-bounds
`allocations[…]` panic at `allocation.rs:974`). **Why:** the `on=3` path serves **two**
data shapes, not one. `data.pos` is offset 4 for a freshly-built scratch, but a
**struct-field offset** (12, in test 502) when a keyed collection is iterated *in place* —
so `data.pos + 4` is not a header slot I control; it reads a neighbouring field as a
store number. There is **no fixed offset** for the source store because the on=3 `data` is
not always my header. The corrected invariant: **the source store_nr must be carried
out-of-band in the iterator STATE, not in the iterated record** — which is precisely the
dormant `on=4` mode the code already documents (`allocation.rs:1000-1002`: *"the runtime
retains the original hash's `store_nr` via the companion iterator-local allocated by
`parse_for_iter_setup`"*). Step 3 is reverted; its intent folds into Step 4 below. *(The
probe that caught this was free: the existing keyed-slice/store-lifetime tests. The
"inert" claim was the over-clean absorption the protocol warns of.)*

**Step 4 — a distinct `on=4` mode (DONE, both backends).** `fill_iter`'s `Hash|Radix` arm
(which always follows the `hash_scratch` substitution → a fresh scratch at `data.pos=4`)
now emits **`on=4`** instead of `on=3`; everything else — including in-place Ordered fields
(pos 12) and index/sorted — stays `on=3`. The header threading Step 3 tried *is* safe here
because `on=4` is a **compile-time** guarantee of a fresh scratch: `build_rec_scratch`
records the source store_nr at header offset 8, and `on=4`'s `step` yields there
(`sourced` flag on the shared `vector::step_ordered`). `iterate` shares the `on=3` cursor
setup (`3 | 4 =>`). A **read-only source** gets its scratch in a fresh dedicated store
(`database(1)`) so the exposed buffer is never claimed into or moved; a writable source
stays co-located (source == scratch store → the golden is byte-identical). The dedicated
store is freed when iteration **completes** (`step` at `pos == i32::MAX`, only when
`source != data.store_nr`; the elements live in the source, untouched) — the interpreter
and the native runtime `step` mirror each other. *Verified: the exposed matrix iterates ==
the writable golden and is leak-clean on a complete walk (both backends); 502/85/the golden
stay green.* The dedicated store is freed on every **loop exit** by the epilogue's
conditional `OpFreeScratch` (Open question A), so complete / break / repeated / nested
exposed iteration are leak-clean. **Sole residual:** a `return` out of the loop bypasses
the epilogue and leaks the current scratch store — caught by the leak check, never a UAF.

**Step 5 — bounded range over an exposed non-hash source (not started).** `on=4` fires only
for `Hash|Radix` (always full iteration → no `ordered_find`). A range over an exposed
`sorted`/`index`/`spatial` stays `on=3`; claiming its scratch in the read-only store would
still panic. Either extend `on=4` to those (threading the source store into `ordered_find`'s
key reads) or restore Step 2's clean error for that cell. *Oracle: bounded-range-over-
exposed either matches its golden or errors cleanly — never a silent wrong element.*

Steps 0-4 are **done** (refactor + clean-error safety net + the `on=4` mode that makes an
exposed hash/radix iterate, leak-clean on a complete walk). Step 3's header idea was
falsified for `on=3` but is sound under the compile-time `on=4` split. Two gaps remain: the
break/return leak (A) and bounded-range-over-exposed (Step 5).

## Open questions (probe before choosing — do not assume)

- **A. Scratch free (RESOLVED for loop exits; `return` residual).** The scratch var
  carries `hash_deps` (borrow → the scope machinery never frees it), so the dedicated
  read-only store had no owner. The fix is a **loop-epilogue conditional free**: `parse_for`
  emits `OpFreeScratch(hash_scratch)` right after the `on=4` loop, which frees the scratch's
  whole store **only when it differs from the source** recorded in the header (offset 8) —
  a no-op for a co-located (writable) scratch, and self-contained (no source-witness var
  needed). It runs on completion AND `break` (break leaves the `v_loop` to the epilogue),
  and once per (re-)entry, so **complete / break / repeated (loop-in-loop) / nested**
  exposed iteration are all leak-clean on both backends. **Sole residual:** a `return` out
  of the loop jumps to function exit, past the epilogue, and leaks the current scratch
  store (leak-check flags it; never a UAF — the freed store holds only rec-nrs, not
  elements). Closing it needs `OpFreeScratch` at *function*-scope exit too, coordinated so
  it doesn't double-free the loop-epilogue's (e.g. `OpFreeScratch` nulling the var it
  frees). Deferred as a smaller follow-up.
- **B. Dedicated store lifecycle.** Persistent-and-reused vs fresh-per-iteration. A
  persistent empty store may trip the "N stores not freed at exit" census — check what
  the census counts (live records vs allocated slots) before choosing. Prefer reuse via
  the existing free-slot mechanism (`src/database/mod.rs:855-882`).
- **C. Should the fast path decouple too?** Tempting to always use a dedicated scratch
  store (uniform, no branch). **Falsified** by failure path #3: it breaks bounded-range
  `ordered_find` for *writable* sources too. Keep the fast path co-located — the
  decoupling is *only* wider than the defect on the read-only path.

## Cross-refs

- Routing feedback: `routing/docs/loft-feedback.md` (2026-07-22 "`expose` UN-RETRACTED";
  the `release`/`expose` bracket is the consumer workaround — routing is unblocked).
- `src/database/allocation.rs:1097-1130` (`build_rec_scratch`), `src/store.rs:641-651`
  (`claim`), `src/state/io.rs:1188-1203` + `src/codegen_runtime.rs:974-991` (the yield
  mirrors), `src/parser/collections.rs:1670-1737` (scratch var typing).
- Discipline: CLAUDE.md § matrix-first; the design-protocol + loft-codegen skills.
