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
offset). `put_stack<DbRef>` stamps; `get_stack<DbRef>` reads the stamp, **consumes it (LIFO
clear)**, and reports `stamped < current`. The LIFO clear is load-bearing: without it a stale
stamp survives a non-DbRef push that reused the offset → false positives (162 reports → 57
after the clear).

### Finding — (c) fingers the crash site

`LOFT_UAF_GEN=1` on the crawler (`src/questtest.loft`) reports **57 distinct stale-reused-ref
reads**, and **`sim.loft:3546` — the exact SIGSEGV site — is among them** (with `3545`
adjacent, plus a cluster at 3482/3483/3901/3947/3951/3955/3969/4003/4020/4050). This
**confirms row 3 + row 8 from the other direction**: the crash is a DbRef read whose backing
store was freed *and re-claimed by a new occupant* between push and pop. The detector converts
the crash from "190-store interleaving, invisible minimally" into a named, line-located
stale-read at the chokepoint (`get_stack<DbRef>`).

**Residual / limitation:** (c) reports only *reads* that flow through `get_stack` (the eval
stack); a DbRef reaching the stack via a **non-DbRef-typed push** (struct-bytes copy) carries
no stamp → it is a false-negative, not a false-positive. So 57 is a *lower* bound on stale
reads, and the detector is safe to trust when it *does* fire (no false alarms) but not as a
completeness oracle.

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
