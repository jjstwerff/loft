---
render_with_liquid: false
---
# @PLN10 — Roadmap to delete `stores.scratch`

The clear path from **today (32 `scratch.push` sites; native + node-wasm +
wasmtime-wasm green — the keystone `W` is SOLVED)** to the
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

### W — curated owned-`String` wrapper-return gate ✅ **DONE** *(keystone)*
> **Solved** (commit). `native_returns_owned_string(name)` helper (`generation/mod.rs`)
> gates the wrapper return type; validated on native + `wasm[node]` + `wasm[wasmtime]`.
> The `tests/generated` "golden regen" was a non-issue (gitignored, written fresh).
> ⚠ rebuild **both** wasm rlibs (`wasm32-unknown-unknown` AND `wasm32-wasip2`) after a
> generation change — a stale wasip2 rlib masks the real state with `E0308`.
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

### N1 — internal text stubs return owned `String` ✅ **DONE** *(landed with W)*
> `i_parse_errors` + `i_json_errors` (`codegen_runtime.rs`) now return owned
> `String`, added to `native_returns_owned_string` in lockstep.  `scratch.push`
> 34 → 32; all three backends green.  (The interpreter-side `n_json_errors` /
> `i_parse_errors` in `native.rs` are still scratch-backed — that's I1's interp half.)

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

### C — chokepoint coverage proof ✅ **DONE (empirically)** *(for the converted producers)*
> **Established this session** by three independent results: (1) **zero** non-`_dest`
> fallback hits across `wrap` + `issues` + `format` (every script/doc + 684
> regression tests) — instrumented the fallbacks with a greppable marker, ran the
> suites, grepped; (2) the **only suspected gap is impossible** — a native method
> can't be fn-ref'd (`v.map(to_lowercase)` → `"second argument must be a function
> reference"`), so only *calls* reach a producer and the chokepoint wraps every
> call position; (3) the chokepoint walk covers **every IR variant** (modeled on
> `substitute_value`), so no value position is left unwrapped.  Conclusion: the
> converted producers' non-`_dest` natives are **dead code**.  (A permanent guard
> test — assert a fallback is never hit — should land with `F`.)

### F — remove interpreter fallback natives *(needs C ✅; merge with D)*
- **Scope:** the non-`_dest` producer bodies in `native.rs` are dead (per C) but
  **can't drop their `scratch.push` cheaply in isolation** — the interp `Str`-on-stack
  ABI needs a backing buffer, and a release stub returning the null sentinel would
  *silently* corrupt if C ever regressed (the suite runs in **release**, so a
  `debug_assert` guard wouldn't fire).  Cleanest: **delete the dead natives + their
  `FUNCTIONS` registration outright at the final D pass** (the loft *def* stays for
  the IR `Call`; only the unreferenced Rust impl goes) — a codegen-time
  `library_names` miss then *loudly* catches any residual emit.  So F is folded into
  **D**, gated on every producer converting first.
- **Effort/risk:** S / low — but **do it with D**, not standalone.  **Label:** `area:codegen`

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
| **M1 — keystone** ✅ | `W` `N1` | DONE — the curated owned-`String` wrapper gate; unblocks the native chain |
| **M2 — producers converted** | `N1 N2 I1 A B` | every producer off scratch (fallbacks aside) |
| **M3 — fallbacks gone** | `C`✅ `F` | coverage proof DONE (fallbacks are dead code); `F` deletes them, folded into `D` |
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
