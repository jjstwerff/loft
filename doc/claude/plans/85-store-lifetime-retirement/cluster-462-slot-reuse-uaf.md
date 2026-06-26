<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Cluster 462 — stale-DbRef-after-slot-reuse UAF (crawler world-gen)

**Source:** [loft#462](https://github.com/loft-lang/loft/issues/462) (sev:high, wa:none,
hit-by:crawler). Surfaced by the crawler dogfood consumer's headless gate.

**Severity (two fields, never conflate):**

| Corruption / panic / hang | Leak |
|---|---|
| 🔴 **SIGSEGV** (interpreter), deterministic, `sim_new_gen_s` @ `src/sim.loft:3546` — the nullable-element struct append `enemies += [mk_enemy(wdef, …)]` | 🔴 **massive** — `LOFT_NO_SLOT_REUSE=1` exhausts the store table (65535 live); pervasive `stores not freed` on `vector<__nullable<Enemy>>` / `<MonsterDef>` / `<ItemDef>` |

A **regression** from the @PLN85 D-own delivery rework ([#457](https://github.com/loft-lang/loft/issues/457)/#459) —
the pre-@PLN85 toolchain produced `QUEST OK`.

---

## The mechanism (accountability table)

Every row is **VERIFIED** (cited probe/run) or **HYPOTHESIZED** (Stage-B must confirm).

| # | Claim | Status | Evidence |
|---|---|---|---|
| 1 | The crash is a **heap fault**, not a stack overflow | ✅ VERIFIED | `ulimit -s unlimited` still SIGSEGVs at the identical site (op=227, sim.loft:3546:40) |
| 2 | The faulting op is `copy_record` reading from a store whose **`free` flag is true** (use-after-free of the copy SOURCE) | ✅ VERIFIED | a pre-copy bounds probe in `do_copy_record` reported `src BAD #189/#190 free=true cap_words=543 tp=76 size=124`, repeatedly, before the fault |
| 3 | The crash is **slot-reuse-dependent**: freeing a store, reusing its slot for a new store, then a live stale DbRef reads/writes the new occupant | ✅ VERIFIED | `LOFT_NO_SLOT_REUSE=1` **eliminates the SIGSEGV entirely** (replaced by store-table exhaustion); slot reuse is the collision channel |
| 4 | Dep accounting is broken in **both** directions in these large functions — premature frees (under-count → the UAF) **and** leaks (over-count → 65535-store exhaustion) | ✅ VERIFIED | the two severity fields above; both observed in the same run |
| 5 | The crash site is the **nullable-element struct append** (`vector<__nullable<Enemy>>` += `[fn()->struct]`), compiled `OpPreAllocVector(246)` + `OpNewRecord(195)` + `OpSetEnum(disc)` + `OpCopyRecord(GetField(elm,8,190),190)` + `OpFinishRecord` | ✅ VERIFIED | introspect of `sim_new_gen_s` at line 3546; all append sites byte-identical in form |
| 6 | The append-site size math is self-consistent (`__nullable<Enemy>` slot = 246 = 8 disc + 238 Enemy); the OOB is **not** a per-site stride/copy mismatch | ✅ VERIFIED | introspect: stride 246, Enemy `size(190)=238` copied at offset 8 |
| 7 | It does **not** reproduce minimally — the trigger is the accumulated slot-reuse interleaving (~190 live stores to collide) | ✅ VERIFIED | probes `462-nullable-append-clean` + `462-borrowed-element-return-clean` (both backends clean); issue author's standalone attempts also clean |
| 8 | The **exact op that prematurely frees** the still-referenced store | ✅ VERIFIED (2026-06-26) | `LOFT_UAF_SRC` (the cheap operand-stack half of the tool gap) pinned it: the dominant driver is `mon_one`'s return aliasing the **local `pool`** (`monsters.loft:258`), freed at `mon_one`'s exit, then bind-copied by `mon_choose` (`:269`). The #306 "return views a local" materialise does not fire for the conditional `chosen = m` view-assign. Full analysis: [over-free-class-study.md](over-free-class-study.md) instance 3 |
| 9 | The premature free is the **borrowed-source over-free class** (same family as the two fixes this session: native bind copy-gate + the implicit-tail borrowed-vector `__fwd`) | ✅ VERIFIED | not the #457/#459 thicket per se — it is *own-vs-borrow re-derived per site*; the chokepoint is the propagated borrow-set (`OWNERSHIP_MODEL`). See the study doc |

---

## Why the standing instruments missed it

- **`LOFT_UAF`** (the use-after-free detector) scans only **live frame variables**
  for a reference to a just-freed slot. Cluster-462's stale reference lives on the
  **operand stack** (the `data` DbRef `copy_record` pops) or **inside a vector
  element** — neither is a frame variable. So `LOFT_UAF` reported only an unrelated
  same-frame tuple case in `ov_hex_world` and never the real free→use pair.
- The **`wrap` leak-gate / ASan / Miri** CI suite runs `tests/scripts/`, not the
  crawler corpus — and the minimal shapes don't trip it (rows 7).

This is the residual the closed @PLN85 outcome-(b) instrument doesn't cover:
**slot-reuse UAF where the survivor reference is not a frame variable, only
reachable at real-consumer scale.**

---

## Tool gaps (part of this cluster's output)

1. **Extend `LOFT_UAF`** (`src/state/debug.rs` `uaf_scan_freed`) to also scan the
   **operand stack** and **live vector elements** for DbRefs into a just-freed slot,
   not only frame variables. This is what converts cluster-462 from "substrate
   rework" into a named free→use pair (closes row 8).
2. **`LOFT_NO_SLOT_REUSE`** — a gated `Stores::disable_slot_reuse` toggle at startup
   (the worker path already has the field). Decisive class-discriminator: if it makes
   a SIGSEGV vanish, the crash is a slot-reuse UAF (row 3). Worth promoting to a
   permanent gated diagnostic alongside `poison_free` / `LOFT_UAF`.

---

## Detectors built (2026-06-26) — three gated diagnostics

Tool-gap #1 is now built as **three** independent, env-gated detectors (all default-off,
zero behavioural effect when unset — verified suite-green). Each catches a different slice
of "a DbRef into a freed slot is still read":

| Flag | Where | What it catches | Soundness |
|---|---|---|---|
| `LOFT_UAF_SRC` | `io.rs do_copy_record` (a) | `copy_record` whose **source store has `free=true`** — the use-after-free at the copy itself | sound (reads the live `free` flag) |
| `LOFT_UAF_REUSE` | `io.rs do_copy_record` (b) | `copy_record` (free=false) where `validate_claims` finds the destination layout already inconsistent — reuse-corruption *visible at copy time* | sound |
| `LOFT_UAF_GEN` | `keys.rs` + `state/mod.rs` `put_stack`/`get_stack` (c) | a DbRef **pushed** to the eval stack, whose slot is later **freed AND reused** (gen bumped) before the DbRef is **popped** and read — the general stale-reused-ref read | sound *in principle* (per-slot generation distinguishes old occupant from re-claimed new); residual false-NEGATIVES only |

**Why the operand-stack free-site scan (the obvious approach) was rejected as UNSOUND:** a
store can be freed and *immediately re-claimed* on the same op (e.g. `s = []` in-place
reinit), so "a stack DbRef points at a slot freed this op" fires ~21856× on legitimate
free-then-reclaim. The **generation** approach (c) fixes this: each slot carries a monotonic
`gen`, bumped on free; a stack DbRef is stamped with its slot's gen at push; a pop is stale
**only if** `stamped < current_gen` (freed *and reused* since push, not merely freed). A
re-claim that didn't bump past the stamp is not flagged.

**(c) implementation:** `SLOT_GEN: Vec<u32>` (per-slot generation, bumped in
`allocation.rs free_named`); `STACK_SHADOW: HashMap<offset,gen>` (the gen stamped at each eval
offset). `put_stack<DbRef>` stamps the slot-gen; **any non-DbRef push CLEARS** the offset;
`get_stack<DbRef>` reads the stamp, **consumes it (LIFO clear)**, and reports `stamped <
current`. The two clears keep the shadow holding a stamp only for a DbRef *live right now* at
that offset — without them an old stamp survives a slot's reuse and reports against an
unrelated later pop (that staleness drove the false positives: 162 → 57 reports).

### Finding — the residual 3546 crash is freed-AND-REUSED, not free=true

Row 2 (`copy_record` source `free=true`) was verified **pre-fix**. After the `mon_one`/`pool`
driver was closed (the `nullable_to_dense_assign` materialise-copy), the **remaining** 3546
SIGSEGV is a *different* shape: `LOFT_UAF_SRC=1` now reports **zero `src BAD`** at the crash —
the source store's `free` flag is **false**. The slot was freed *and already re-claimed* by a
new, **smaller** occupant; the `OpCopyRecord` at 3546 reads Enemy-size (238 bytes, offset 8)
from it → out-of-bounds → SIGSEGV. So post-`mon_one`, cluster-462's live crash is a
**reused-slot** UAF, not a freed-slot UAF — the fix target is the over-free of the
nullable-element append source (`enemies += [mk_enemy(...)]`), whose return store is freed
while the append's copy-source DbRef is still live.

### Finding — (c) is a NOISY diagnostic; it does NOT cleanly pin 3546

**This corrects an earlier over-claim** (the engineering-rigor lesson: I trusted the first
coherent reading). `LOFT_UAF_GEN=1` reports ~56 stale reads, but they split sharply by
**gen-delta** (`current − stamped`):

- **Small delta (1–~30): trustworthy.** A DbRef plausibly sat on the eval stack across a few
  intervening frees of its slot. These cluster at `sim.loft` **3193, 3222, 3226, 3901, 3947,
  3951, 3969, 4003** — the **`sim_descend` `ns`-struct-return region** the roadmap already
  named as the *second* driver — plus scattered library sites.
- **Huge delta (hundreds–50000+): residual false positives.** A delta of 9419 means the slot
  was freed+reused 9419× "between push and pop" of one DbRef — implausible for an eval-stack
  temporary. These are offsets whose stamp went stale despite the clears (a DbRef reaching the
  offset by a path `put_stack`/`get_stack` don't see — e.g. a multi-word struct copy writing
  DbRef-shaped words at `base+k`). The `gen 0 at push` reports are the same class.

**`sim.loft:3546` itself reports delta = 9419** — it sits in the *residual* bucket, NOT a
trustworthy hit. So **(c) does not finger the actual SIGSEGV site**; it reliably surfaces the
*sim_descend* driver but is blind-with-noise on 3546. Pinning 3546 needs a **store-identity**
instrument (watch the specific source slot's alloc/free/reuse), not the offset-shadow.

**Limitation summary:** (c) is sound *in mechanism* (per-slot gen) but its offset-keyed shadow
leaks on DbRef movement outside `put_stack`/`get_stack` → both false-negatives (unstamped
reads) and large-delta false-positives. **Trust small-delta reports; treat large-delta as
noise.**

### Detector (d) — the stale-interior-claim guard that ACTUALLY pins 3546

Where (a)/(b)/(c) all missed 3546, a **phase trace** inside `do_copy_record` (markers before
`remove_claims` / `copy_block` / `copy_claims`) pinned the fault precisely: the SIGSEGV is in
**`remove_claims(&to)`** — *before* the copy — not in the copy itself. op=227 IS `copy_record`
(confirmed via the `fill.rs` OPERATORS table), but it faults clearing the **destination's old
content**, not reading the source. That is why every source-side detector read "clean".

`remove_claims` walks the dst record's owned children; for a struct it recurses into each
field, and a **text** field (`tp==5`) does `store.delete(cur)` on the stored text-record
pointer. `Store::delete` reads the record header at `cur` — and in release the bounds
`debug_assert` is gone, so a garbage `cur` past the store end faults. Detector (d)
(`allocation.rs remove_claims`, gated `LOFT_UAF*`) validates `cur < capacity_words` before the
delete; on a bad pointer it **names the field and SKIPS the delete** (leak, not crash).

**Result:** with (d), the crawler **runs to completion (`QUEST OK`, exit 0)** instead of
SIGSEGV. (d) names the exact fault:

```
[uaf-claim] remove_claims: TEXT field at store #535 rec=418 pos=6390 holds STALE pointer
cur=3407872 past store end (cap_words=2956; slot free=false known_type=196 rec_pos_valid=true)
```

**Root signal (what the slot-state tells us):** the dst slot is **live** (`free=false`), the
offset **is in bounds** (`rec_pos_valid=true`), `known_type=196` — only the text-pointer
*values* (glyph/key/name, at consecutive offsets 6390/6394/6398) are garbage. So this is **not**
a freed/relocated dst; it is a **live destination record whose payload text-pointers were never
validly initialised (or were over-freed)**, which `remove_claims` then walks. The dst is the
final target of a chained return-copy (`#609 → #608 → #535`).

**Two candidate roots (next step — discriminate with a write-watch on `#535 rec=418`):**
1. *Uninitialised fresh slot* — `claim`/`resize` zero only under `zero_claim_enabled()` (opt-in,
   off in release; `resize`'s in-place grow path absorbs an adjacent free block and never zeroes
   it), so a freshly-exposed slot holds stale free-tree bytes that `remove_claims` walks before
   the copy writes them.
2. *Prior over-free* — an earlier delivery wrote a borrowed text ref into `#535`'s payload and
   that target was freed, leaving the pointer dangling.

The fix target is upstream of `remove_claims` (don't tear down a never-initialised dst, or zero
the slot before the copy-bracket runs `remove_claims`). (d) is the diagnostic + a temporary
safety net, **not** the root fix — it is gated off by default, so released runs still crash.

(Both `LOFT_NO_SLOT_REUSE` and the throwaway operand-stack scan were used as probes and
reverted; (a)/(b)/(c) are the kept, committed diagnostics.)

---

## Probes

| File | Shape | Result | Role |
|---|---|---|---|
| `probes/462-nullable-append-clean.loft` | `vector<__nullable<Big>>` += `[fn(structarg)->33-field-text-struct]` in a loop | ✅ clean both backends | negative control / over-reach marker |
| `probes/462-borrowed-element-return-clean.loft` | + source is a borrowed nullable-vector element returned from a fn (`mon_choose_habitat` shape) | ✅ clean both backends | negative control |
| (in-crawler) `loft --interpret <libs> src/questtest.loft` | full world-gen | 🔴 SIGSEGV @ sim.loft:3546 | the only reproducer — does not shrink |

**Reproducer (in-crawler):**
```sh
cd crawler
BL=$(for d in bundles/*/ bundles/*/items/; do printf -- "--lib %s " "$d"; done)
loft --interpret --lib ../loft-libs-core-main/ --lib ../loft-libs-world/ $BL src/questtest.loft
```

---

## Roadmap (per cluster)

1. **[S]** Promote the two tool gaps (extend `LOFT_UAF`; gate `disable_slot_reuse`).
2. **[M]** With the extended `LOFT_UAF`, name the premature-free op on the crawler
   run (closes row 8) — read it against the #457/#459 diff (closes row 9).
3. **[M/L]** Fix the dep miscount at its chokepoint in the @PLN85 delivery/adopt
   path; graduate a real-scale regression (the minimal probes can't guard it —
   needs a crawler-corpus or a synthetic ~200-store slot-reuse stress).
4. Address the coexisting **leak** (row 4) separately — closing the SIGSEGV while
   leaks persist is the false-fix trap this plan's two-severity rule guards against.
