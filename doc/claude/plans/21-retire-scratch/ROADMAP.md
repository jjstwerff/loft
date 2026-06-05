---
render_with_liquid: false
---
# @PLN10 — Roadmap to delete `stores.scratch`

The clear path from **today (34 `scratch.push` sites, both backends green)** to the
**goal (the field deleted; Goal E for strings — no global text buffer)**, as a set
of dependency-ordered issues.

Read [`01-destination-passing-design.md`](01-destination-passing-design.md) first —
it is the design + the build evidence each issue below builds on.

> **These "issues" live HERE, not in GitHub.**  They are plan sub-tasks WE
> decomposed and fix ourselves in sequence — the DAG below tracks them and they
> close as the plan advances.  A GitHub Issue is earned by being *surfaced* (a
> defect found in the wild, especially one that blocks or another repo hits), not
> by being *planned*; filing our own decomposition would just pollute the tracker
> with rows nobody outside this plan needs.  Promote one to a real Issue only if it
> escapes the plan (blocks unrelated work / handed off).  See
> [ISSUE_TRACKING.md § The split](../../ISSUE_TRACKING.md#the-split--what-lives-where).

---

## The goal & the acceptance bar

**Goal:** delete `Stores::scratch` (`src/database/mod.rs`), the dead
`clear_scratch` (`fill.rs`), the no-op `OpClearScratch` (`default/02_files.loft`),
and the dead per-`Line` emission (`state/codegen.rs:329`).

**Acceptance:** `grep -rn 'scratch.push' src/` = **0**, then the field deletes and
its absence is the compile-time guard.  `tests/scripts/192`–`194` stay green on
**all three backends** (interpreter / native / wasm) throughout.

**Done so far** (13 commits): the interpreter synth-temp chokepoint + the native
cell-ABI producers.  `39 → 34`.

---

## The dependency graph

```
            ┌─────────────────────────────────────────────┐
   W (keystone) ──► N1 ──► N2 ─┐                            │
            │                  │                            ▼
   I1 (interp) ────────────────┼──► C ──► F ──────────────► D  (GOAL)
            │                  │                            ▲
   A (null) ──────────────────┼────────────────────────────┤
            │                  │                            │
   B (Phase B) ───────────────┴────────────────────────────┘
```

`W` is the **keystone** — it unblocks every native `Str→String` conversion.
`D` (field delete) needs **all** of `N1 N2 I1 A B F` at zero.

---

## The issues

### W — curated owned-`String` wrapper-return gate *(keystone — ROOT CAUSE FOUND, de-risked)*
- **Root cause (investigated this session):** on `wasm32-wasip2`, EVERY text native
  gets a generated *wrapper function* (the native backend inlines them).  A blanket
  `def.code → String` flip changes the wrapper *return type* but not the *bodies*,
  which still produce `Str`/`&str` (`#rust` templates like `trim → &str`,
  unconverted producers like `parallel-buf`/`as_text` → `Str`) → `E0308`.  It is
  **wrapper/body type consistency, NOT** the `Deref`/`Str`-field rewrite first
  guessed.  (`wasm[node]`'s `--html` build emits fewer wrappers, which is why it
  goes green once the introspector bodies match.)  See design doc § native-backend.
- **Scope:** add `native_returns_owned_string(name)` (shape of `is_text_dest_native`)
  gating `mod.rs:2134 → "String"` to ONLY producers whose `codegen_runtime` body we
  convert; add each name **in lockstep** with its body conversion.  `#rust` / `as_text`
  / cdylib stay `Str`.  Then regenerate `tests/generated/*.rs` (cached `-> Str`).
- **Accept:** `wasm_library_suite` (node + wasmtime) + `native_library_suite` green
  with the introspectors converted.
- **Effort/risk:** **M / med** (was "high" — now a scoped helper + golden regen, no
  bridge rewrite).  ⚠ rebuild the `wasm32-unknown-unknown` rlib before trusting a
  wasm failure (a stale rlib panics "build panicked" before the real error).
- **Label:** `area:wasm` `area:codegen`

### N1 — internal text stubs return owned `String` *(needs W)*
- **Scope:** condition `mod.rs:2134` on `def.code == Null && text → "String"`;
  convert the introspector bodies (`i_parse_errors`, `i_json_errors` in
  `codegen_runtime.rs`) — the two-family rule (owned-stub `String` vs user-fn
  buffer-view `Str`).  Already proven on native; W makes it WASM-safe.
- **Accept:** `scratch.push` −2; all three backends green.
- **Effort/risk:** S / low (once W lands).  **Label:** `area:codegen`

### N2 — cdylib FFI text wrap → owned `String` (Phase A.5) *(needs W, N1)*
- **Scope:** the `needs_text_wrap` emitted body (`mod.rs:2698`) + the interpreter
  cdylib bridge (`extensions.rs:651,894` — `bridge_push_str` / `push_loft_str`).
- **Accept:** `scratch.push` −3; cdylib libraries green on native + wasm.
- **Effort/risk:** M / med.  **Label:** `area:codegen` `area:wasm`

### I1 — remaining interpreter producers → dest-passing *(independent; native half needs W)*
- **Scope:** per producer, the proven Build-3 pattern — a `_dest` variant +
  `is_text_dest_native` entry (interp) and an owned-`String` `codegen_runtime`
  body (native, gated on W).  **Audit each for null-safety first.**  The set
  (from the producer audit): `n_ymd_days_ago`, `n_store_memory`,
  `struct_to_json_dispatch`, `n_parallel_buf_get_text(_native)`,
  `n_ws_client_message`, `n_pack_take`, `os_variable` (`database/format.rs`).
- **Accept:** each producer dest-passes in value position (matrix), both backends.
- **Effort/risk:** M / low-med (mechanical × N; some may be null → route to A).
  **Split** the interp side (ship now) from the native side (gate on W).
  **Label:** `area:codegen`

### A — `as_text` null-carrying return *(design)*
- **Problem:** `as_text` returns **null** for a non-string `JsonValue`; an owned
  `String` / a dest buffer can't carry null (empty buffer == `""`, not null).
- **Scope:** decide the null representation (native text already has a sentinel —
  `STRING_NULL`); convert `n_as_text` (interp) + `t_9JsonValue_as_text` (native)
  under it, or formally keep it scratch-backed + document why.
- **Accept:** `j.as_text() ?? x` semantics preserved on all backends.
- **Effort/risk:** S-M / med (it's a small design).  **Label:** `area:codegen`

### B — generic-specialisation text wraps (Phase B) *(independent)*
- **Scope:** the 8 `emit.rs` `stores.scratch.push(...); Str::new(...)` emissions for
  bounded-generic text returns (the @P205 family).  Thread a `RefVar(Text)` work
  buffer through generic specialisation instead (the `text_return` machinery).
- **Accept:** `scratch.push` −8 (emit.rs); the `p205_no_str_new_of_local_in_corpus`
  test updates to the new shape.
- **Effort/risk:** M / med.  **Label:** `area:codegen`

### C — chokepoint coverage proof *(needs I1)*
- **Scope:** prove **no** value-position text-producer call bypasses the chokepoint
  (so the non-`_dest` interpreter fallback natives are dead code).  A boundary
  matrix over the call positions + a guard test.
- **Accept:** a documented argument + a test that fails if a bypass exists.
- **Effort/risk:** M / **med-high** (the verification arc — the real residual risk).
  **Label:** `area:codegen`

### F — remove interpreter fallback natives *(needs C)*
- **Scope:** delete the non-`_dest` producer bodies in `native.rs` (the bulk of its
  16 `scratch.push` sites — the `t_4text_*`/`n_kind`/`n_to_json`/… fallbacks now
  unreachable per C).
- **Accept:** `scratch.push` drops by the fallback count; suite green.
- **Effort/risk:** S / low (deletion, once C holds).  **Label:** `area:codegen`

### D — delete `Stores::scratch` (THE GOAL) *(needs N1 N2 I1 A B F)*
- **Scope:** with `scratch.push` == 0, delete the field + `clear_scratch` +
  `OpClearScratch` decl + the `codegen.rs:329` emission; the field's absence is the
  guard.
- **Accept:** `grep scratch src/` clean; `192`–`194` green all backends; field gone.
- **Effort/risk:** S / low (the payoff).  **Label:** `area:codegen`

---

## Critical path & milestones

| Milestone | Issues | Meaning |
|---|---|---|
| **M1 — keystone** | `W` | the WASM `Deref` fix; unblocks every native conversion |
| **M2 — producers converted** | `N1 N2 I1 A B` | every producer off scratch (fallbacks aside) |
| **M3 — fallbacks gone** | `C F` | coverage proof + delete the dead non-`_dest` natives |
| **M4 — GOAL** | `D` | field deleted |

**Longest chain (critical path):** `W → N1 → N2 → D` on the native side, and
`I1 → C → F → D` on the interpreter side — these two run in parallel; `D` waits on
the slower.  `A` and `B` are independent and can land any time before `D`.

**Recommended order:** `W` first (it's the keystone and the only high-risk
investigation) → then `N1`+`I1` in parallel → `N2`+`A`+`B` → `C` → `F` → `D`.

**If `W` proves too costly:** the interpreter chain (`I1 → C → F`) is fully
independent of it.  Banking the interpreter side (its fallbacks removed) is a
clean partial — the field can't delete, but interpreter scratch traffic reaches
zero, which is most of the Goal-E memory win for the reference backend.
