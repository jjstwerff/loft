# 57 — Vector store-lifetime watermark (@P393)

## Status (REQUIRED)

| Stage | Status |
|---|---|
| A — Probe catalogue | ✅ complete (9 probes, both backends) |
| B — Mechanism investigation | 🟡 2/2 clusters *characterised by trace*; root-cause source-reading not started |
| C — Fix design (OPTIONAL) | ⏸️ pending Stage B — design call (deliberate scope-free vs missed last-use-free) |
| D — Implementation | ⏸️ pending Stage C |

**What triggered this.** Filed as [@P393](../../../PROBLEMS.md) on 2026-06-01 while
verifying the @P390 self-slice fix: `LOFT_STORES=warn loft --tests tests/scripts/11-vectors.loft`
emits ~14 `possible leak at alloc #N` warnings as the active-store count climbs to
~40 (max=40), yet the test **passes the exit leak gate** (`tests/wrap.rs:276`). So
this is a *during-run high-watermark*, not an exit leak. The investigation's job:
characterise whether the climb is a real per-operation delayed-free or benign
function-scope accumulation, and across which vector shapes.

**Stage-A verdict (verified by `LOFT_STORES=log` traces — see [RESULTS.md](RESULTS.md)):**
the `possible leak` warnings are a **false positive for an actual leak** — every store
frees at function-scope exit (11-vectors trace is `aaaa…(44 allocs)…ffff…(42 frees)`,
**zero interleaving**; exit gate passes). The high watermark is real and has two
verified contributing mechanisms (clusters I + II below). It is a **memory-watermark /
heuristic-false-positive** concern, **not** a correctness/leak concern.

## Goal (REQUIRED)

Decide and ship the watermark reduction for function-local vector lifetime: either
(a) free dead store-backed locals at last-use instead of scope-end (cluster I), and/or
(b) stop the literal-init temp from pinning to scope (cluster II) — plus a heuristic
fix so `LOFT_STORES=warn` stops flagging legitimately-long functions. The "do nothing,
it's benign" outcome is on the table and must be argued explicitly, not assumed.

## In-plan vs spinoff policy (default: in-plan)

Both clusters stay in-plan — they share the function-scope free-sweep surface (a
last-use-free pass would touch both). **One orthogonal finding is flagged for spinoff**
(not in-class): probe 08's original `total += (base + [i]).len();`×35 form panics with
a codegen slot mismatch (`Incorrect var total[504] versus 496`, `src/state/codegen.rs:2669`)
— a slot-assignment bug in functions with many inline-concat `+=` statements, unrelated
to store lifetime. Recorded in [RESULTS.md](RESULTS.md) § Spinoff; candidate standalone
P-issue (user's call — not filed, per the investigation-plan convention that in-plan
findings stay in the catalogue).

## Cluster catalogue (REQUIRED)

| ID | Cluster | Severity (corruption / leak) | Backend asymmetry | Probes | Doc |
|---|---|---|---|---|---|
| I | Named store-backed locals freed at **scope-end**, not last-use → dead locals hold their store for the rest of the function | none / none (exit-safe) — watermark O(locals) | both (interp 44, native 42) | 02, 07, 09, 11-vectors | [cluster-I-scope-end-batching.md](cluster-I-scope-end-batching.md) |
| II | `a = [literal]` allocates **2** scope-pinned stores (literal temp not freed/reused after copy into the local) → doubles the watermark | none / none (exit-safe) — watermark ×2 constant | both | 07, 09 | [cluster-II-literal-init-double-alloc.md](cluster-II-literal-init-double-alloc.md) |

Neither cluster is a leak. Transient *unbound* temps (slice-in-format, probe 08) free/
reuse correctly at statement-end — they are **not** part of the problem.

## Probe suite (REQUIRED)

| File | Shape | Cluster | Status |
|---|---|---|---|
| `01-baseline-single-vector.loft` | one small vector, build+read | reference | passes — baseline (watermark ~2) |
| `02-loop-local-integer.loft` | fresh `vector<integer>` local per loop iter | I (contrast) | passes — store **reused in-place**, watermark flat (4 allocs) |
| `03-loop-local-text.loft` | per-iter `vector<text>` | I (contrast) | passes — flat |
| `04-loop-nested-vector.loft` | per-iter `vector<vector<integer>>` | I (contrast) | passes — flat |
| `05-multi-fn-returns.loft` | helper builds+returns vector value ×20 | I (contrast) | passes — per-call store freed at return |
| `06-slice-ops-loop.loft` | per-iter slice into a new local | I (contrast) | passes — flat |
| `07-sequential-named-locals.loft` | 35 **typed** named vector locals | I + II | **watermark 72** (≈2× per local), all freed at scope-end |
| `08-sequential-transient-temps.loft` | 35 unbound slice-in-format temps | — (control) | passes — watermark **4**; temps free/reuse at statement-end |
| `09-untyped-named-locals.loft` | 35 **untyped** named vector locals | II | **watermark 72** — identical to 07; annotation is *not* the cause |

`tests/scripts/11-vectors.loft` is the field repro (not copied into `probes/`): 44 allocs,
watermark 44 interp / 42 native, all freed at scope exit.

## Reference ↔ problem pairings (REQUIRED if probes ≥ 5)

| Problem | Reference | What the diff reveals |
|---|---|---|
| 07 (35 named locals, watermark 72) | 02 (loop local, watermark 4) | A loop body is a scope that exits each iteration → store **reused**; 35 sibling statements in one scope each allocate + pin. The watermark is driven by *distinct scope-lived bindings*, not by total allocations. |
| 07 (watermark 72) | 08 (35 unbound temps, watermark 4) | Binding to a named local is what pins the store to scope-end; the same value produced as an *unbound* temp frees at statement-end. Cluster I is specifically about **named** locals. |
| 09 (untyped, 72) | 07 (typed, 72) | The `: vector<integer>` annotation does **not** change the count → cluster II's doubling comes from the literal-init temp, not the type annotation. |

## Tool gaps (OPTIONAL but recommended)

| Tool | Status | Used for |
|---|---|---|
| `LOFT_STORES=log` | Verified-essential | Full alloc/free/dec_rc trace (`src/database/allocation.rs:131`). The alloc/free *interleaving* (`aaaa…ffff…` vs `afafaf…`) is the whole diagnosis. |
| `LOFT_STORES=warn` | Verified-suitable but **coarse** | Threshold hardcoded at `active > 30` (`allocation.rs:142`) — not configurable, and fires on any legitimately-long function. A Stage-C tool fix should make it watermark-relative or raise the floor. |
| Per-alloc source label | **Gap** | Interpreter allocs log an empty `name` field, so the trace can't say which alloc is which local/temp. Native attaches names via `OpFreeRef(var_name)` only on free. Labelling interp allocs would let a future probe attribute the watermark per-binding. |

## Status & next-session roadmap (REQUIRED)

| Cluster | Mechanism status | Action needed | Effort |
|---|---|---|---|
| I | 🟢 Characterised by trace (scope-end batching, both backends). Root cause 🤔: is store-free decoupled from variable last-use by design (LIFETIME.md scope model) or a missed last-use-free? | Read `src/scopes.rs` + `src/state/codegen.rs` free-emission; confirm where store-free is anchored (scope-end vs live-interval end). | M |
| II | 🟢 Characterised by trace (2 stores per literal-init local, both pinned; annotation-independent). Root cause 🤔: literal temp copied into the local's store rather than *becoming* it; the temp's free is deferred to the scope sweep. | Read the `=`/init codegen path for `local = [literal]`; find the literal-temp alloc + its free site. | S–M |

**Recommended sequence.** Stage B reading (cluster II first — smaller surface, and the
fix likely halves the watermark on its own) → decide Stage C (do-nothing vs last-use-free
vs literal-temp-reuse) → if fixing, cluster II then cluster I, one commit each. The
**quickest user-visible win** is the `LOFT_STORES=warn` heuristic floor (raise/relativise
the threshold) — it removes the false-positive noise regardless of whether the watermark
itself is touched, and is XS.

**Open design question for the user (Stage C).** loft's scope-based freeing
([LIFETIME.md](../../../LIFETIME.md)) frees locals at scope exit by design. Cluster I asks
whether store-backed locals should additionally free at last-use. That trades a smaller
watermark for added free-emission complexity (and interacts with dep-tracking / aliasing —
the same surface as PLAN51/52). This is a deliberate language-design call, not a
mechanical fix — left for the user.

## Relationship to the store-lifetime/aliasing class

Kindred to (all closed/finished) [PLAN51 hidden-buffer-aliasing](../../finished/51-hidden-buffer-aliasing/),
[PLAN52 value-block-borrow-cleanup](../../finished/52-value-block-borrow-cleanup/),
@P383, @P390, @P391 — the recurring store-lifetime / DbRef-lifetime family that maps onto
soundness-floor A in [GOALS.md](../../../GOALS.md). Distinct from those: @P393 is a *watermark/
efficiency* finding with **no correctness failure**, where the others were corruption or
dangling-ref bugs. It is filed as its own plan (not folded into PLAN51) because PLAN51 is
finished.
