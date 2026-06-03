<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Stage A — probe matrix results

34 probes × {interp, interp+vec4, native, native+vec4}.  Runner:
`probes/run_matrix.sh release`.  `rc=139`→SIGSEGV; `rc=101`→Rust panic;
FAIL-COMPILE→parser/codegen rejects; FAIL(1)→native produced no correct output.

## Headline findings

1. **`--vec4` is SAFE but INSUFFICIENT.** Forcing the vector-handle stride to 4
   changed **no** probe's pass/fail on either backend (only the prealloc operand
   `16→4`).  No shape relied on the wrong stride (no two-wrongs-cancel
   breakage), AND no broken shape was fixed by it.  ⇒ the 16/8 vector-handle
   stride divergence is **latent** (memory-waste / fragility), not the cause of
   any observed failure.  The active bugs are other mechanisms.  This is the
   probe-first payoff: "align to 4" would have shipped a no-op-for-users change.
2. **Single-sentinel crash is PERVASIVE** — every `single` nested shape crashes
   on interp (8/8 contexts), far wider than #262's filed "only 3-deep copy".
3. **Narrow-element nested reads silently corrupt** — `i32` reads wrong values,
   `boolean` reads empty inner vectors.  No crash, no diagnostic: silent
   data loss (the highest-severity value category, S).

## Full matrix

```
probe                            | interp       | interp+vec4  | native       | native+vec4
---------------------------------+--------------+--------------+--------------+-------------
01-flat-int-baseline             | PASS         | PASS         | PASS         | PASS
02-vv-int-literal-read           | PASS         | PASS         | PASS         | PASS
04-vv-single-2deep               | SIGSEGV      | SIGSEGV      | FAIL(1)      | FAIL(1)
05-vvv-single-copy               | SIGSEGV      | SIGSEGV      | FAIL(1)      | FAIL(1)
06-vvv-int-copy                  | PASS         | PASS         | PASS         | PASS
20-2d-bool-litread               | corrupt[]    | corrupt[]    | FAIL(101)    | FAIL(101)
20-2d-char-litread               | FAIL-COMPILE | FAIL-COMPILE | FAIL-COMPILE | FAIL-COMPILE
20-2d-float-litread              | PASS         | PASS         | PASS         | PASS
20-2d-i32-litread                | corrupt(0)   | corrupt(0)   | FAIL(101)    | FAIL(101)
20-2d-int-litread                | PASS         | PASS         | PASS         | PASS
20-2d-single-litread             | SIGSEGV      | SIGSEGV      | FAIL(1)      | FAIL(1)
20-2d-struct-litread             | PASS         | PASS         | PASS         | PASS
20-2d-text-litread               | PASS         | PASS         | PASS         | PASS
20-2d-tuple-litread              | PASS         | PASS         | PASS         | PASS
30-3d-bool-copy                  | PASS         | PASS         | PASS         | PASS
30-3d-char-copy                  | FAIL-COMPILE | FAIL-COMPILE | FAIL-COMPILE | FAIL-COMPILE
30-3d-float-copy                 | PASS         | PASS         | PASS         | PASS
30-3d-i32-copy                   | PASS         | PASS         | PASS         | PASS
30-3d-int-copy                   | PASS         | PASS         | PASS         | PASS
30-3d-single-copy                | SIGSEGV      | SIGSEGV      | FAIL(1)      | FAIL(1)
30-3d-struct-copy                | PASS         | PASS         | PASS         | PASS
30-3d-text-copy                  | PASS         | PASS         | PASS         | PASS
40-ctx-int-write                 | PASS         | PASS         | PASS         | PASS
41-ctx-single-write              | SIGSEGV      | SIGSEGV      | FAIL(1)      | FAIL(1)
42-ctx-int-structfield           | PASS         | PASS         | PASS         | PASS
43-ctx-single-structfield        | SIGSEGV      | SIGSEGV      | PASS         | PASS    ⟵ backend asymmetry
44-ctx-int-fnreturn              | PASS         | PASS         | PASS         | PASS
45-ctx-single-fnreturn           | SIGSEGV      | SIGSEGV      | FAIL(1)      | FAIL(1)
46-ctx-int-comprehension         | PANIC-CONST  | PANIC-CONST  | FAIL-COMPILE | FAIL-COMPILE
47-ctx-single-comprehension      | PANIC-CONST  | PANIC-CONST  | FAIL-COMPILE | FAIL-COMPILE
50-4d-int                        | PASS         | PASS         | PASS         | PASS
51-4d-single                     | SIGSEGV      | SIGSEGV      | FAIL(1)      | FAIL(1)
60-fnref-direct                  | PASS         | PASS         | PASS         | PASS
61-fnref-call-returned           | PANIC-CONST? | PANIC-CONST? | FAIL-COMPILE | FAIL-COMPILE (#263)
```

(`corrupt[]` / `corrupt(0)` print as FAIL-COMPILE/FAIL in the raw runner — the
assertion fails on a silently-wrong read; re-classified here after isolation.)

## Cluster assignment

| Cluster | Shapes | Mechanism | `--vec4`? |
|---|---|---|---|
| I — vector-handle stride (16/8/4/4) | (latent — no probe fails on it) | construction prealloc 16 / read 8 vs true 4 | flips operand, no behaviour change |
| II — single-NaN-sentinel SIGSEGV | 04,05,20-single,30-single,41,43,45,51 | `single` null `0x7FC00000` read as wild rec-id; **all 8 single contexts crash** | no effect (orthogonal) |
| III — narrow-element nested read | 20-i32 (`vv[0][1]→0`), 20-bool (`vv[0]→[]`) | nested read loses the narrow element width → wrong stride → silent corruption | no effect (element isn't a vector) |
| IV — nested comprehension CONST_STORE | 46, 47 | `[for … { […] }]` writes to a read-only const store (`store.rs:1386`); #248 family | no effect |
| V — call-returned fn-ref (#263) | 61 | storing a CALL-returned fn-ref into a collection (60 direct passes) | no effect |

## Verified isolations (not probe artifacts)

- **III/i32** — `/tmp/i32n.loft`: `vv[0]=[1,2]` (print path correct) but `vv[0][1]=0`
  (indexed read wrong); flat `vector<i32>` reads `fv[1]=2` correctly.  ✅ real.
- **III/boolean** — `/tmp/booln.loft`: flat `vector<boolean>` len=3 correct;
  nested `len(vv)=2` but `len(vv[0])=0` (inner reads empty).  ✅ real.
- **II/single 2-deep literal** — construct-only (`/tmp/p04_construct.loft`)
  SIGSEGVs with no read involved → crash is at construction, not read.  ✅ real.
- **IV** — `46` panics `Write to read-only store … (locked by CONST_STORE init)`.
- **V** — `61` (call-returned) fails where `60` (direct named) passes → matches #263.

## Backend asymmetries

- `43-ctx-single-structfield`: interp SIGSEGV but **native PASS** — the single
  sentinel reaches a rec-id read on interp's struct-field path but not native's.
- All single shapes: interp SIGSEGV vs native FAIL(1) (no crash, no correct
  output) — different surfacing, same root sentinel.

## Unclassified / pending isolation

- **char** (20,30): `Field access not supported on type character` at `vv[0][0]`
  — may be a real type-inference loss through nesting or a probe-shape quirk;
  needs a flat-vs-nested `character` isolation before claiming a bug.

## What the matrix establishes for Stage B

- Cluster II (single) and III (narrow) are the **user-visible** targets; both are
  silent-or-crashing and neither is the vector-handle stride.
- Cluster I is real but latent; decide in Stage C whether to unify it anyway
  (cleanliness) by making `--vec4` permanent or fixing the resolvers.
- The `--vec4` lever's job is done as a *measurement*: it proved the handle
  stride is safe to unify and is NOT the active fault.

## Stage B — root-fix iterations + probe hardening

Fix loop (apply at a resolution site → re-run matrix → measure):

| # | change | matrix delta |
|---|---|---|
| 1 | `typedef.rs::fill_database`: resolve a nested-vector element via the #250-proven `db_type` recursion (instead of the level-collapsed `type_elm`→`known_type`) | **+1**: `43-ctx-single-structfield` SIGSEGV→PASS (interp); **no regressions** |
| 2 | `vector_of` inner-narrow | none — bypassed (see below) |
| 3 | `typedef.rs` inner-narrow | none — collapse is upstream (see below) |

Only iteration 1 landed (the others were reverted as dead).  Verified mechanism
for why 2/3 were dead — the **i32 corruption is a nested-literal narrow-coercion
bug, not a resolver bug**: for `vv: vector<vector<i32>> = [[1,2]]`, `lhs_known`
is `None` (plain vector) so `vector_of(in_t)` runs, but `in_t` is already
`vector<integer>` — the integer literals `1,2` are inferred wide and **not
coerced to the declared `i32`** through the nesting (flat `vector<i32>=[1,2]`
*does* coerce, which is why flat works).  Construction strides by 8, the read
(declared `i32`) by 4 → `vv[0][1]=0`.  ✅ Verified via `LOFT_LOG=static`
(`OpNewRecord(vv,65)`, element var typed `vector<integer>`; read `OpGetInt4`@4).

### Probe hardening (answering "can a value coincidentally hide a case?")

Weak probes (small values `1,2,3`; index `[0]` is stride-independent; no length
checks) can pass despite a stride bug.  Hardened probes `70-76` use distinctive
byte-distinct values, assert **every** index + `len()` at each level, and
cross-row independence:

| strong probe | result | verdict |
|---|---|---|
| 70 int / 72 struct / 73 tuple / 74 text / 75 float | PASS both backends | **genuinely correct** — not value-coincidence false passes |
| 71 i32 | fail / panic | confirmed broken (strong values) |
| **76 bool** | **interp SIGSEGV** | **severity correction** — the weak `20-2d-bool` showed only an empty read; with 3 alternating rows it is a **crash** (memory corruption), not silent corruption |

⇒ Two outcomes: the int/struct/tuple/text/float passes are real (so the +1/no-
regression claim is trustworthy for them), and **boolean is upgraded to a crash**
(was mis-filed as silent-corruption).  Probe lesson folded into the suite: every
new probe asserts all indices + lengths with distinctive values.
