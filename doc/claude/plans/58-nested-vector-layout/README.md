<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 58 — Nested-vector layout (vector-of-vectors stride & sentinel reconciliation)

## Status

| Stage | Status |
|---|---|
| A — Probe catalogue | ✅ 34-probe matrix run on both backends ±`--vec4` ([RESULTS.md](RESULTS.md)) |
| B — Mechanism investigation | 🟡 5 clusters identified; II/III mechanisms pinned, I latent, IV/V map to #248/#263 |
| C — Fix design | ⏸️ pending Stage B |
| D — Implementation | ⏸️ pending Stage C |

**Stage-A outcome (the surprise):** the `--vec4` / `LOFT_VEC4=1` force-to-4 lever
is **safe but insufficient** — across all 34 probes × both backends it changed
*no* pass/fail (only the prealloc operand `16→4`).  So the vector-handle stride
divergence (16/8) is **latent**, not the cause of any observed failure.  The
user-visible breakage is elsewhere: a **pervasive single-NaN-sentinel SIGSEGV**
(every `single` context, far wider than #262) and **silent narrow-element
corruption** (`i32` reads wrong, `boolean` reads empty).  Probing first stopped
us from shipping an "align to 4" fix that would have closed nothing for users.

`vector<vector<…>>` is a **load-bearing data structure** the language relies on
(map tiles, matrices, adjacency lists, comprehension results) — not a new
feature.  This investigation treats it as a stability class: close the whole
family rather than spot-fix one reported crash.  Going in, four sibling bugs are
already known — #246 (compound-assign, fixed-pending-merge), #250 (3-deep type-id
divergence, fixed-pending-merge), #262 (3-deep single crash, open), #263
(call-returned fn-ref into collection, open) — which is the signal that the fix
surface spans several interacting mechanisms.

## Goal

Reconcile the **four divergent strides** a nested-vector element handle reports
today — construction `16`, index-read `8`, runtime deep-copy `4`, true handle
`4` — to a single source of truth, and close the orthogonal **single-sentinel**
crash class, so every (depth × element-type × context) shape of
`vector<vector<…>>` works identically on `--interpret`, `--native`, and WASM.

## The four-way stride disagreement (the thesis)

A vector element that is itself a vector is stored as a 4-byte `u32` rec-id
handle.  Three different code paths compute that element's size and get three
different answers:

| Site | Resolver | Reports | File |
|---|---|---|---|
| Construction (prealloc) | `type_def_nr(elem)` → `FieldValue` alias | **16** | `parser/vectors.rs:1640` |
| Index read (`vv[i]`) | `type_elm(vec)` → bare `vector` builtin | **8** | `parser/fields.rs:677` |
| Runtime deep-copy / `OpCopyRecord` | `stores.vector(...)` chain | **4** | `database/allocation.rs`, `#250` fix |
| True handle | — | **4** | (a `u32` rec-id) |

The strides are baked into IR operands at parse time, so the parser sites
(`16`/`8`) reach **both** backends.  Some shapes survive today only because two
wrong strides cancel (e.g. the outer `vv[0]` read uses the linked-record
`OpVectorRef` path — already 4 — so the read-side `8` never bites *that* form).
Mapping which shapes the cancellation saves vs. which it breaks is Stage A's job.

## In-plan vs spinoff policy (default: in-plan)

Findings surfaced during this investigation are **fixed in-plan** and recorded
in the cluster catalogue — not filed as separate issues (the probes + cluster
docs already document them).  Exceptions per
[`_INVESTIGATION_TEMPLATE.md`](../_INVESTIGATION_TEMPLATE.md): a true edge case,
or a fix surface large enough to need its own plan.  The four pre-existing
GitHub issues (#246/#250/#262/#263) are the *seed* shapes; on closure the still-
open ones either close via the fix or get their forward home per the template.

## Cluster catalogue

Full evidence + the 34-cell matrix: [RESULTS.md](RESULTS.md).

| ID | Cluster | Severity (crash / corruption) | Backend | `--vec4` | Doc |
|---|---|---|---|---|---|
| I | Vector-handle stride divergence (16/8/4/4) | **latent** — no probe fails on it | both | flips operand, no behaviour change | `cluster-I-stride-divergence.md` |
| II | ~~Single-NaN-sentinel read as wild rec-id~~ **✅ CLOSED** — @P380 handle-zero hoisted to all construction paths; 7 cells SIGSEGV→PASS both backends; regression `tests/scripts/183` | was SIGSEGV, all `single` contexts | both | n/a | `cluster-II-single-sentinel.md` |
| III-a | ~~Narrow-int nested literal (`i32`/`i16`/`u8`)~~ **✅ CLOSED** — `parse_item` now propagates the declared element type into the inner literal (`vectors.rs:1797`); regression `tests/scripts/184` | was silent corruption | both | n/a | (see RESULTS.md) |
| III-b | char nested — **out of scope** (flat `vector<character>` indexing is broken generally, not nesting); boolean — **read-side** handle stride (own cluster) | — | — | — | (see RESULTS.md) |
| boolean | ~~Outer vector-of-vectors **handle stride** = inner scalar size (1) not 4 → handles overlap~~ **✅ CLOSED** — parse-time `known` fix (`new_record`) + read-stride clamp (`fields.rs`); regression `tests/scripts/185` | was corruption→crash | both | n/a | (see RESULTS.md attempt 4) |
| IV | ~~Nested comprehension → CONST_STORE write~~ **✅ CLOSED** — deep-copy (`OpCopyRecord`) + element-type `known` (`vectors.rs`); 46/47/91/105/106 PASS; regression `tests/scripts/186` | was panic | both | n/a | (see RESULTS.md; distinct from #248) |
| V | Call-returned fn-ref into collection | **panic** (#263) | both | **out of scope** — general (flat too), not nesting | (see #263) |

Headline reframe from the matrix: the size-alignment thesis (Cluster I) is real
but **latent** — the user-visible faults are Cluster II (pervasive `single`
SIGSEGV, far wider than #262's filed scope) and Cluster III (silent narrow-element
data loss).  `char` nested vectors also reject (`Field access … on character`) —
unclassified pending a flat-vs-nested isolation (see RESULTS.md).

### Suspected single ROOT cause (working hypothesis)

These are likely **not five independent clusters** but symptom-fallout of **one
root cause**: the type system conflates `vector<T>` with `vector<vector<T>>` (the
#250 type-id family).  A nested-vector element is typed as *"some generic
vector"* rather than *"`vector<T>` specifically"*, so the strides (16/8), the
deref type-id (e.g. `65` in the i32 trace), and the single/narrow read paths all
read the wrong number out of the same conflation.  `--vec4` patched one readout
(the handle stride) and changed no behaviour ⇒ the stride is downstream; the
type-id resolution is the root.  **Plan of record:** fix the root type-id
resolution, re-run this matrix to measure which symptoms close, then probe deeper
where it moves.  `--vec4` is retained as the measurement toggle for that loop.

## Probe suite

34 self-contained `.loft` files in `probes/`; each asserts then prints
`PASSED <name>`.  Runner: `probes/run_matrix.sh release` (interp/native ±`--vec4`,
per-cell timeouts).  The matrix spans depth `{2,3,4}` × element `{integer, single,
float, text, boolean, i32, char, struct, tuple, fn-ref}` × context `{literal,
+=copy, read, write, struct-field, fn-return, comprehension}`.  Full result table
in [RESULTS.md](RESULTS.md).

Real-library extraction landmark: `tests/scripts/182-deep-nested-vector-copy.loft`
(the #250 regression) covers integer/text 3-deep + struct-field; its header flags
`vector<vector<vector<single>>>` as the cluster-II family.

## Tool gaps

| Tool | Status | Used for |
|---|---|---|
| `--vec4` / `LOFT_VEC4=1` (`src/vec4.rs`) | **New (this plan)** — TEMPORARY | Force every parser-computed nested-vector element stride to its true 4-byte handle size so the sweep measures "does aligning strides close the class?" against one toggle.  Scoped to `Type::Vector` elements; no-op when already 4.  Wired at the 4 parser stride sites (`vectors.rs:1640`, `fields.rs:74`, `fields.rs:677`, `collections.rs::element_store_size`).  **Remove when the resolver fix lands.** |
| `LOFT_LOG=static` | Verified-suitable | Confirm the `16→4` operand flip in the IR dump. |
| `timeout 20` watchdog | Verified-essential | SIGSEGV probes self-terminate with `rc=139`. |

## Status & next-session roadmap

| Cluster | Mechanism status | Action needed | Effort |
|---|---|---|---|
| I | 🟢 Thesis verified (four-stride table; `16→4` flip proven) | Finish shape matrix to find every form the read-side `8` / prealloc `16` actually breaks (vs. two-wrongs-cancel survivors); decide resolver fix vs. keep clamp | M |
| II | 🟡 Mechanism known (NaN `0x7FC00000` read as rec-id); scope wider than #262 (2-deep literal crashes) | Pin crash site (construction `new_record` single-handle write); extend @P380 OpSetInt4-zero to the deeper/literal forms | M |

**Sequence:** (1) complete Stage-A matrix on both backends; (2) Cluster II first
(it's a hard SIGSEGV — highest user harm), extend the @P380 sentinel-zero fix;
(3) Cluster I — choose between making the lever permanent vs. fixing the
resolvers to return a size-4 type-id, then retire `--vec4`.  Fixes land **one
cluster per commit, pushed before the next** (template § Fix-application
discipline).

## See also

- [`tests/scripts/182-deep-nested-vector-copy.loft`](../../../../tests/scripts/182-deep-nested-vector-copy.loft) — #250 regression, real-shape landmark.
- `src/parser/vectors.rs` (construction, `new_record`, @P380), `src/parser/fields.rs` (index read), `src/parser/collections.rs` (`element_store_size`).
- `src/database/allocation.rs` (`copy_claims`, runtime stride), `src/database/types.rs` (`db_type`, `size`, resolvers).
- GitHub #262 (cluster II seed), #246/#250 (fixed-pending-merge), #263 (fn-ref handle, candidate cluster).
