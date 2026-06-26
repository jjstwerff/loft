<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# NEXT SESSION — start here (cluster-462 leak fix)

> ## ✅ THE ADOPT-AND-RE-RETURN LEAK IS FIXED (2026-06-26, commit `cafe98a0`)
>
> The task this doc describes is **done**. Fix at the NRVO chokepoint
> (`control.rs nrvo_collapse_tail_set` + new `nrvo_collapse_defining_call`): mark each collapsed
> **vector** work-ref `skip_free` (no orphan alloc, no free) and extend the collapse to the
> `t = f(); t += …; t` merge shape. **Crawler interp 531→0, native 752→216**; both backends +
> full suite green; regression `tests/leak_cases/clean/p462_adopt_rereturn_vector.loft`. Detail:
> [cluster-462-slot-reuse-uaf.md](cluster-462-slot-reuse-uaf.md) roadmap item 4.
>
> **Still open under #462 (roadmap item 5):** the native-only `MonsterDef×216` **record** leak —
> a DIFFERENT mechanism (the `mon_*` borrowed-view shape, pre-existing on `main`). Keep #462 open.
> The rest of this doc is the original (now-historical) task description.

Cold-start handoff after the 2026-06-26 session that fixed the #462 crash and root-caused the
#462 leak. Branch: **`fix-crawler`** (everything pushed; no open PR). Read order: this file →
[cluster-462-slot-reuse-uaf.md](cluster-462-slot-reuse-uaf.md) → the repros below.

---

## TL;DR

- **#462 CRASH = FIXED** (commit `0ccc756c`): `Store::resize`'s in-place grow now zeroes the
  absorbed region. Crawler completes `QUEST OK` on both backends; suite 2542-green; regression
  `store.rs::resize_in_place_zeroes_absorbed_region`.
- **#462 LEAK (adopt-and-re-return) = FIXED** (commit `cafe98a0`, see banner above). The residual
  native record leak (different mechanism) keeps #462 open.
- Issue #462 is intentionally **kept open** (two-severity rule): crash fixed, vector leak fixed,
  the native record leak remains. Commits use `Refs #462`, NOT `Fixes #462`.

---

## THE TASK: fix the adopt-and-re-return leak

### The bug (verified, minimal)

```loft
fn base() -> vector<Aaa> { t: vector<Aaa> = []; t += [Aaa{a:1,name:"a"}]; t }
fn m()    -> vector<Aaa> { t = base(); t }    // ← LEAKS 1 store per call, BOTH backends
// g = base();   directly → CLEAN. The wrapper that adopts-then-re-returns is what leaks.
```

A function that binds a vector-returning call into a local and re-returns that local leaks
exactly one store per call. `game_items()` / `game_monsters()` in the crawler are this shape
(`t = item_table(); … merge …; t`) → 518 + 13 = **531 leaked** stores.

### Repros (committed)

- `probes/leak-462/adopt-rereturn-single.loft` — single call, leaks 1 (smallest).
- `probes/leak-462/adopt-rereturn-leak.loft` — loop, leaks N.
- `probes/leak-462/merge-sibling-adopt-leak.loft` — the exact `game_items()` merge shape.

Run (interp leak check):
```sh
LOFT_STORES=warn loft --interpret probes/leak-462/adopt-rereturn-single.loft
#   → "Warning: 1 stores not freed ... main_vector<__nullable<Aaa>>×1"
LOFT_LEAK_SITES=1 LOFT_STORES=warn loft --interpret <prog>   # groups leaks by alloc site → line
LOFT_NATIVE_LEAK_CHECK=1 loft --native <prog>                # native leaks the SAME 1/call
```

### Root cause (from the broken bytecode — `loft introspect`)

`m()` mints a **dead `__ref_1` work-ref** instead of collapsing onto its `__retbuf`:
- **native**: `var___ref_1 = OpDatabase(cell, var___ref_1, 68)` is emitted at the top of `m`,
  then `var_t = n_base(cell, var_t)` delivers into the retbuf `var_t` — so `var___ref_1` is
  allocated, never used, never freed → leak.
- **interp**: `__ref_1` stays `null` (no alloc) but the returned local is mis-tagged
  `t["__ref_1"]`, so the caller's `OpFreeRef` indirects through a phantom owner and the real
  returned store is never freed.

The NRVO collapse fails to elide the intermediate work-ref when the **returned local is itself
an adopted call result** (`t = base(); … ; t`). The call already delivers into the retbuf; the
extra `__ref_1` buffer + its ownership tag are spurious.

### Fix target

`src/parser/control.rs` — the `block_result` return-delivery collapse (`Delivery`/`RefDelivery`
selectors, `ref_return`, `nrvo_collapse_tail_set`). When a returned local is first-assigned from
a **buffer-returning call** (`!return_adopts_fresh_store()`), the call should deliver into the
fn's `__retbuf` directly with correct ownership — **no intermediate `__ref_1`**. Likely also
`src/scopes.rs` (the dep/owner tagging at ~1055–1102, the `paired_witness` region — that handles
the *over-free* mirror; the leak is the same chain under-freeing).

### Method (MANDATORY — this is the suite-regressing area)

Load the **loft-codegen skill** first. The gate: do NOT edit the compiler until you have the
WORKING bytecode captured beside the BROKEN one for this shape, proven on BOTH backends.
- BROKEN reference: `loft introspect probes/leak-462/adopt-rereturn-single.loft` (the dead
  `__ref_1`).
- WORKING reference: the byte-shape of `g = base()` directly (clean) is the target — the wrapper
  should emit the same delivery, no extra buffer.
- Build a one-fn-per-path corpus (adopt-and-return, fresh-builder, direct-call, sibling-merge,
  match-arm-return) and prove byte-identical on the paths you DON'T mean to change.
- Verify: `LOFT_STORES=warn` (interp) + `LOFT_NATIVE_LEAK_CHECK=1` (native) leak-free on the
  repros, AND `./scripts/find_problems.sh --bg` full suite green, AND the crawler still
  `QUEST OK` on both backends.

### Boundary (what must STAY working — do not regress)

Clean today, must remain clean: `g = base()` (direct), fresh builders (`t=[]; …; t`), inline
temporaries (`len(build())`), single adopted local used-not-returned, match-arm vector returns
(the cluster-II graduated `85-*` regressions).

---

## Diagnostics built this session (all gated, default-off, kept as standing tools)

| Flag | What it does |
|---|---|
| `LOFT_UAF_SRC` | copy-source `free=true` UAF (detector a) |
| `LOFT_UAF_REUSE` | copy-source structurally-invalid / reused (detector b) |
| `LOFT_UAF_GEN` | per-slot generation + eval-stack shadow stale-read (detector c) — NOISY; trust small gen-delta only |
| (in `remove_claims`) | stale-interior-claim guard (detector d) — fires under `LOFT_UAF*`; names a dst field with an OOB text-ptr and skips the delete |
| `LOFT_WATCH_STORE=<n>` | after each copy into store `<n>`, report an OOB text-ptr write + source |
| `LOFT_LEAK_SITES=1` | group exit leaks by allocation site (`created_at` → source line) |

NB: `LOFT_UAF_GEN` over-reports (offset-shadow leaks on non-`put_stack`/`get_stack` DbRef moves);
the exit-warning "N stores" is **N type-groups**, not N stores (the `×count` is the real count).

---

## Crawler reproducer

```sh
cd /home/lima.guest/crawler
BL=$(for d in bundles/*/ bundles/*/items/; do printf -- "--lib %s " "$d"; done)
LOFT_TIMEOUT=180 loft --interpret --lib ../loft-libs-core-main/ --lib ../loft-libs-world/ $BL src/questtest.loft
#   → QUEST OK (crash fixed); exit warning still lists the 531-store leak
```

## Broader @PLN85 open work (after the leak)

- **Cluster A** (#429 borrowed-view return over-free) — in flight.
- **Cluster C / H10** — fold `copy_claims`/`validate_claims`/construct onto the
  `for_each_owned_child` keystone (~53 `Parts::` arms across 3 dispatchers). Next pass after A.
- **#460 / #461** — crawler-wave native cdylib-dispatch + mixed-mode struct-arg corruption.
