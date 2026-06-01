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
not change the store count → cluster II's 2×-per-local doubling is from the literal-init
temp, independent of annotation.

## Spinoff (orthogonal — not the store-lifetime class)

Probe 08's **original** form (`total += (base + [i]).len();` ×35, unbound inline-concat
accumulated into `total`) panics:

```
thread 'main' panicked at src/state/codegen.rs:2669:9:
Incorrect var total[504] versus 496 on n_main
```

A codegen slot-assignment mismatch in functions with many inline-concat `+=` statements
— unrelated to store lifetime. Probe 08 was rewritten to the slice-in-format form (which
runs clean and answers the temp-lifetime question). Candidate standalone P-issue; **not
filed** (investigation-plan findings stay in the catalogue — but this one is orthogonal to
the class, so it is surfaced here for the user to triage). Minimal repro saved nowhere yet;
reproduce with 35× `total += (base + [i]).len();` in one `fn main()`.

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
