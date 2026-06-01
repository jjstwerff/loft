# RESULTS — @P393 vector store-lifetime watermark

Stage A probe runs. All under `LOFT_STORES=log` / `LOFT_STORES=warn` (threshold
`active > 30`, hardcoded `src/database/allocation.rs:142`). Interpreter unless noted.
Binary: `./target/release/loft` (release, built from current `ci-hygiene`/`debugging` tree).

## The diagnostic axis: alloc/free interleaving

`LOFT_STORES=log` prints `+ alloc #N` / `- free #N` per store event. The **order** is
the whole diagnosis:

- `afafaf…` (interleaved) → stores free promptly; flat watermark; healthy.
- `aaaa…ffff…` (all allocs, then all frees) → every store is held until one batch
  free at scope-end; watermark = number of live bindings.

## Probe matrix

| Probe | Backend | allocs | frees | event seq | max watermark | result |
|---|---|---|---|---|---|---|
| `01-baseline-single-vector` | interp | — | — | — | ~2 (no warn) | `baseline sum=6` |
| `02-loop-local-integer` (20 iters) | interp | **4** | 2 | `aaaaff` | flat (~2) | `loop-int total=630` — store **reused in-place** across iters |
| `03-loop-local-text` (20 iters) | interp | low | — | flat | flat | `loop-text n=60` |
| `04-loop-nested-vector` (20 iters) | interp | low | — | flat | flat | `nested n=40` |
| `05-multi-fn-returns` (×20 calls) | interp | low | — | flat | flat | `multi-fn total=1140` — per-call store freed at return |
| `06-slice-ops-loop` (20 iters) | interp | low | — | flat | flat | `slice-loop total=440` |
| `07-sequential-named-locals` (35 typed) | interp | **72** | 70 | `a×72 f×70` | **72** | `seq-locals total=595` — ≈2 stores/local, all freed at scope-end |
| `08-sequential-transient-temps` (35 unbound slices) | interp | **4** | 2 | `aaaaff` | **4** | `temps done` — unbound temps free/reuse at statement-end |
| `09-untyped-named-locals` (35 untyped) | interp | **72** | — | `aaaa…ffff…` | **72** | `untyped total=595` — identical to typed 07 |
| `10-struct-vectors` (10 `vector<Item>`) | interp | **22** | 20 | `aaa…fff` | 22 | 2/local — struct elements **inline**, no per-element store |
| `11-comprehension-init` (10 `c=[for…]`) | interp | **22** | 20 | `aaa…fff` | 22 | 2/local — comprehension init **also doubles** |
| `12-concat-init` (10 `c=a+b`) | interp | **16** | 14 | `aaa…fff` | 16 | ~1/local — concat result **becomes** the local; **no double** |
| `13-slice-init` (10 `t=base[a..b]`) | interp | **14** | 12 | `aaa…fff` | 14 | ~1/local — slice materialises into the local; **no double** |
| `14-reassignment` (1 local ×10 reassign) | interp | **14** | 12 | `aaa…fff` | 14 | ~1/assign, **old store not freed on overwrite** (cluster III) |
| `15-if-block-locals` (10 `if`-block locals) | interp | **22** | 20 | `aaa…fff` | 22 | 2/local, **all pinned to fn exit** — non-loop blocks do not free at block-end |
| `11-vectors.loft` (field repro) | interp | **44** | 42 | `aaaa…(44)…ffff…(42)` | **44** | passes; all frees at scope exit, zero interleaving |
| `11-vectors.loft` (field repro) | **native** | — | — | — | **42** | passes; same watermark climb under `--native` |

## Key traces

**11-vectors (interp), `LOFT_STORES=log` — full event sequence collapsed to a/f:**

```
aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa   <- 44 allocations, NONE freed yet
ffffffffffffffffffffffffffffffffffffffffff     <- then 42 frees in one burst at scope exit
```

Zero `dec_rc` events. Last 12 events are all `- free` with `active` counting down
14→3 — confirming the burst is at function teardown.

**11-vectors (native), `LOFT_STORES=warn`:** climbs to `max=42` with the same
`possible leak at alloc #N` warnings — the watermark is a property of the program/scope
model, not a backend artifact.

**Probe 02 (loop), `LOFT_STORES=log`:** `aaaaff` — only 4 stores allocated for a
20-iteration loop. The loop body's vector store is allocated once and **reused in place**
(clear+refill) each iteration. This is *why* loops never accumulate, and the sharp
contrast against probe 07.

**Probe 07 vs 09 (typed vs untyped, both 72):** the `: vector<integer>` annotation does
not change the store count → cluster II's 2×-per-local doubling is from the init temp,
independent of annotation.

## Allocs per local by init/scope shape

10 locals per probe; overhead ≈ 2 (CONST store #1 + the main locals/schema store).
Slope confirmed at N=20 for the two key shapes (literal 42 = 2 + 2×20; concat 26 ≈ 2 + 1×20).

| Shape | Probe | allocs/local | Cluster | Note |
|---|---|---|---|---|
| literal `[..]` | 07/09 | **2** | II | doubles — fresh temp then bind |
| comprehension `[for…]` | 11 | **2** | II | doubles — same materialise-then-bind path |
| struct vector `[Item{…}]` | 10 | **2** | II | doubles; elements inline (no per-element store) |
| `if`-block local | 15 | **2**, pinned to fn exit | I | non-loop blocks do not free at block-end |
| concat `a + b` | 12 | **~1** | II (ref) | result store *becomes* the local |
| slice `v[a..b]` | 13 | **~1** | II (ref) | materialises into the local (@P390 fast path) |
| reassign `v = […]` | 14 | **~1**, old store pinned | III | overwrite does not free the prior store |
| loop body | 02 | store **reused** in place | I (ref) | flat watermark |

Takeaway: cluster II is specific to the **materialise-then-bind** init forms
(literal / comprehension / struct); concat and slice already do it right (1×) and are the
reference for the fix. The watermark = Σ over distinct scope-lived bindings of their
per-binding multiplier; only loop bodies escape it (reuse).

## Orthogonal codegen bugs — FILED separately as @P394 + @P395

Edge-probing @P393's aliasing question surfaced two codegen bugs in a different subsystem
(slot / stack-position assignment, not store lifetime). Both are scope-pinned and
root-caused, so neither needs a plan — they are filed as standalone P-issues and fixed
directly:

- **[@P394](../../../PROBLEMS.md)** — `b = a` (new local ← bare vector var) leaves the LHS
  on slot `u16::MAX` → `Incorrect var b[65535] versus N` at `codegen.rs:2669` (or hang, or
  silent-empty `b` + source corruption). Root: the @P292 branch's `uses(var_nr) > 0` guard
  (`src/parser/expressions.rs:1333`) excludes first-assignment. Edge matrix: only a bare
  local/param vector-var RHS; field / element / call / slice / concat RHS all work + copy.
  Both backends. Workaround `b = a[..]`.
- **[@P395](../../../PROBLEMS.md)** — `(v + [x]).len()` (a method call on an inline concat
  temp) in an assignment RHS mis-accounts the stack by a constant 8 bytes → silent garbage
  (len 4 → `8589934592`), escalating to the same `codegen.rs:2669` assert across several
  statements. Root: the concat-temp guard (`src/parser/vectors.rs:39`, "assign to a variable
  first") fires for direct/print/index positions but misses the assignment-RHS
  method-receiver. Concat-specific (literal/slice-temp receivers work). Workaround: bind
  the concat first.

Probe 08 (the original lead) was rewritten to the slice-in-format form (runs clean, answers
the temp-lifetime question). The aliasing answer that fell out of this probing — vectors are
copy-semantics on expression-assignment — is what resolved cluster I/III fix-safety (see
[cluster-III-reassignment-pin.md](cluster-III-reassignment-pin.md) § Fix-safety).

## Edge-probe results (probes 16–19)

| Probe | Finding | Effect on the model |
|---|---|---|
| `16-generality-struct-text` | 20 struct+text locals → **12** allocs (inline, no separate store) | Cluster I is **collection-specific**, not all-locals |
| `17-assignment-aliasing` | `b = a` tripped @P394; via the working `b = a[..]` idiom: independent **copy** | Resolved fix-safety (copy semantics) + surfaced @P394 |
| `18-parse-init` | `as vector<T>` init → **1×**/local | Joins concat/slice in the "1× / init done right" group |
| `19-while-loop` | `while` loop → store **reused**, flat | "Only loops reuse" extends to all loops; non-loop blocks pin |

## Reproduce

```bash
# field repro
LOFT_STORES=warn  ./target/release/loft --tests     tests/scripts/11-vectors.loft   # ~14 warnings, gate passes
LOFT_STORES=log   ./target/release/loft --interpret  tests/scripts/11-vectors.loft \
  | grep -E '\+ alloc|\- free' | sed -E 's/.*alloc.*/a/;s/.*free.*/f/' | tr -d '\n'   # aaaa…ffff…

# probe sweep (watermark per shape)
for p in doc/claude/plans/future/57-vector-store-watermark/probes/*.loft; do
  echo "$p: $(LOFT_STORES=log ./target/release/loft --interpret "$p" 2>&1 \
    | grep -oE 'max=[0-9]+' | grep -oE '[0-9]+$' | sort -n | tail -1)"
done
```
