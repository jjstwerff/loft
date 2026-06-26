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

(Both were used as throwaway probes this session and reverted; promote them when the
cluster's fix work starts.)

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
