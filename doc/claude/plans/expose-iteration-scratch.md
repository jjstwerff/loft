<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Design — iterate an exposed (read-only-pinned) keyed collection

A slice of [@PLN105](105-ffi-deliver-layout-bridge.md) (the FFI deliver / layout
bridge). Surfaced by the `routing` consumer (`docs/loft-feedback.md`, 2026-07-22,
"`expose` UN-RETRACTED"). **Status: Steps 0-2 DONE (golden + refactor + clean-error
safety net); Step 3 FALSIFIED and reverted (see below); Steps 4-5 = a self-contained
`on=4` mode slice, not yet built.** This doc is the single source of truth; it leads
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

**Step 4 — a distinct `on=4` read-only-source iteration mode (the real fix, now a
protocol change, not a header tweak).** Revive `on=4`: for a read-only (exposed) source,
build the scratch in a dedicated writable store and iterate it via a mode whose
**iterator state also carries the source `store_nr`**; `step` for `on=4` yields
`DbRef{store_nr: source, rec, pos: 8}`. Normal iteration stays `on=3` (yields
`data.store_nr`, unchanged — so 502/85 and the golden are untouched). Wire it through
`parse_for_iter_setup` (mode + state), `iterate`/`step` on both backends, and the scratch
lifecycle (Open question A). *Oracle: the full matrix — exposed-hash-full == writable
golden (value), exposed bytes unchanged (invariant), census to baseline (leak), both
backends; AND 502/85/the golden stay green (the `on=3` path is byte-for-byte unchanged).*
Replaces Step 2's error for the hash-full case. **This is genuinely M — a bytecode-mode
change in the store-lifetime subsystem — and should be built as its own slice with the
matrix above, not squeezed onto the end of Steps 1-2.**

**Step 5 — bounded range over an exposed non-hash source.** `on=4`'s `iterate` setup, for
a *range*, still needs `ordered_find` to read record keys from the source store (not the
scratch store). Either thread the source `store_nr` there too, or keep Step 2's clean
error for that cell. *Oracle: bounded-range-over-exposed either matches its golden or
errors cleanly — never a silent wrong element.*

Steps 1 and 2 are **done and shippable** (the mirror-collapse refactor and the clean-error
safety net — no more silent wasm hang). Step 3 was falsified and reverted; its goal moves
into Step 4, which is now a self-contained `on=4` mode slice gated behind the full oracle
set. Step 5 closes the scope gap.

## Open questions (probe before choosing — do not assume)

- **A. Scratch free (the load-bearing one).** The scratch var carries `hash_deps` at
  parse time (borrow → never freed); the read-only store home is a *runtime* fact. Two
  candidates, decide by probing the leak oracle: **(a)** emit an explicit
  free-of-scratch at loop-scope exit targeting the scratch's *actual* store (works for
  both co-located and dedicated); **(b)** a dedicated scratch store that reclaims per
  build. (a) is preferred (composes with nesting); (b) risks failure path #4. *Falsify
  whichever you pick with the nested-exposed-iteration cell + the leak oracle.*
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
