---
render_with_liquid: false
---
# @PLN10 — Roadmap to delete `stores.scratch`

The clear path from **today (29 `scratch.push` sites; native + node-wasm +
wasmtime-wasm green — keystone `W` SOLVED, `codegen_runtime` now scratch-free,
the cdylib FFI codegen wrap now owned-`String`)** to the **goal (the field
deleted; Goal E for strings — no global text buffer)**, as a set of
dependency-ordered issues.

Read [`01-destination-passing-design.md`](01-destination-passing-design.md) first —
it is the design + the build evidence each issue below builds on.

> ## ▶ Next session — start here
> **Done (✅):** the interpreter chokepoint, every native `codegen_runtime`
> producer (that file is now ZERO `scratch.push`), the keystone `W`, the
> coverage proof `C`, and **N2a** — the cdylib FFI **codegen** wrap (the @P244
> `output_native_direct_call` text path now returns owned `String`, not a
> scratch-backed `Str`).  **39 → 29** grep hits (**28 real** statements — one
> `pre_eval.rs:556` hit is a comment); native + interp green.
>
> ### How far from zero — the count only drops at three steps
> The hard infrastructure is built; the rest is proven-pattern application, **one**
> genuinely-new mechanism, and a mechanical delete.  Crucially, **I1 (converting
> the live interp producers) drops the count by ZERO** — it adds `_dest` variants
> but leaves the old bodies in place; its job is to make D's final delete *safe*.
>
> | Step | Removes | Count | Nature |
> |---|---|---|---|
> | **now** | — | **28** | emit 8 / native 16 (5 dead + 11 live) / format 2 / extensions 2 |
> | **Phase B** | emit.rs −8 | 20 | central Return conversion, direction-proven (matrix-gated) |
> | **N2b** | extensions.rs −2 | 18 | needs the new null-aware interp primitive |
> | **D + F** | native −16, format −2 | **0** | mechanical delete + field gone |
>
> ### The cleanest ordering (sequence by MECHANISM, not by issue-label)
> Phase B, I1, N2b are mutually **independent** and all gate D, so order is driven
> by risk + subsystem coherence.  Each phase drives a coherent backend region to
> zero; difficulty rises across phases:
>
> 1. **Phase 1 — I1-nonnull** *(proven, fast, count-neutral)*: give the non-null
>    live producers (`ymd_days_ago`, `store_memory`, `struct_to_json`,
>    `parallel_buf`, `pack_take`…) a `_dest` variant + `is_text_dest_native` entry
>    — exactly the Build-3 template the 5 already-converted producers use.  Per
>    producer: convert + matrix + commit.  Unlocks (part of) D's final delete.
> 2. **Phase 2 — the null-aware interp primitive → I1-null + N2b** *(THE one novel
>    piece)*: a single mechanism — "materialise possibly-null dynamically-produced
>    text on the interp stack without scratch" — feeds THREE consumers that all
>    share it: the null-carrying producers (`os_variable`), interp `as_text`, and
>    the cdylib bridge (`extensions.rs` `bridge_push_str`/`push_loft_str`, which
>    can return null too).  A dest buffer can't carry null (the `as_text`
>    exclusion); the native side already proved the escape hatch (the `STRING_NULL`
>    sentinel as an owned value — see `A`), so the interp equivalent is *unbuilt,
>    not impossible*.  Design it ONCE.  Reaches **interp scratch = 0** (−2, and
>    every `native.rs` body now provably dead).
> 3. **Phase 3 — Phase B** *(central, highest blast radius)*: the 8 `emit.rs`
>    `Value::Return`/`wrap_result` wraps → owned `String`.  ⚠ Serves THREE roots —
>    `no_work_buffer` (@P205 generic monomorph, excluded from `text_return` at
>    `definitions.rs:811`), `returns_local_text` (@P321e), `returns_ncc_block`
>    (@PLAN52 `??`) — all must stop emitting `scratch.push` to zero the grep.
>    Owned-`String` is the direction, but it touches every native text fn, where
>    the **wrapper/body consistency hazard** (the W trap) bites: a fn with MIXED
>    returns (one buffer-view `Str::new(work_buf)`, one owned local) can't be one
>    clean signature.  **Build a return-shape matrix first.**  Do it last before D
>    so a thorny Phase B doesn't block the banked interp wins (−8).
> 4. **Phase 4 — D + F** *(mechanical payoff)*: delete `Stores::scratch` + the 16
>    `native.rs` dead bodies + 2 `format.rs` helpers + dead `clear_scratch` /
>    `OpClearScratch` + the `state/codegen.rs:329` emission (and reword the
>    `pre_eval.rs:556` comment).  The loft *def* stays for the IR `Call`; a
>    `library_names` miss then loudly catches any residual emit.  Gated on 1–3
>    reaching zero.
>
> **The single insight:** I1-null and N2b *look* like separate issues but share one
> missing primitive (null-carrying owned-text on the interp stack) — fusing them
> into Phase 2 designs it once and surfaces the null wrinkle before it bites two
> places.  Isolating the two genuinely-hard pieces (Phase 2's null primitive,
> Phase 3's mixed-return) into their own matrix-gated phases keeps Phase 1
> (proven-mechanical) and Phase 4 (the payoff) low-risk.
>
> ⚠ **Env:** after ANY `src/generation/` or `codegen_runtime.rs` change, rebuild
> **all three** rlibs before trusting a backend, or a stale one fakes failures:
> ```
> cargo build --release --lib --bin loft
> cargo build --release --target wasm32-unknown-unknown --lib --no-default-features --features random
> cargo build --release --target wasm32-wasip2          --lib --no-default-features --features random
> ```
> And confirm native flakes serially (the dep-rlib `rustls`/`webpki`/`ureq`
> parallel-build race is NOT a regression).

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
its absence is the compile-time guard.  `tests/scripts/192`–`195` stay green on
**all three backends** (interpreter / native / wasm) throughout.

**Done so far:** the interpreter synth-temp chokepoint, all native `codegen_runtime`
cell-ABI producers, the keystone `W`/`N1`, the coverage proof `C`, `A` (native
`as_text` null), and `N2a` (cdylib FFI codegen wrap).  `39 → 28` real statements.

---

## The dependency graph

```
   W (keystone)✅ ─► N1✅ ─► N2a✅ ─────────────────────────┐
                                                            ▼
   A (null)✅ ──────────────────────────────────► C✅ ─► F ─► D  (GOAL)
                                                    ▲
   ── remaining (each gates D, mutually independent) ──
   Phase 1: I1-nonnull ────────────────────────────┤   (count-neutral; makes
   Phase 2: null primitive → I1-null + N2b ─────────┤    D's delete safe / −2)
   Phase 3: B (central Return emit) ────────────────┘   (−8)
```

`W` was the **keystone** — it unblocked every native `Str→String` conversion (done).
`D` (field delete) needs **Phase B + N2b + I1** at zero, then deletes the dead
bodies (`F`) + the field.  Order them by mechanism — see § the cleanest ordering.

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

### N2a — cdylib FFI text wrap → owned `String` (codegen half) ✅ **DONE**
> **Solved** (commit `f4e18da6`).  `output_native_direct_call`'s `needs_text_wrap`
> body now returns `String::from_utf8_unchecked(_bytes)` (owned), the wrapper
> signature flips to `-> String`, and the `--html` graceful stub returns
> `String::new()` — all gated on the structural `!def.native.is_empty()` signal
> (disjoint from the curated `native_returns_owned_string` name set).  The caller
> bridges `String` → `Str` via `Deref` (@P304).  **`scratch.push` 30 → 29.**
> - **Why it was clean (vs Phase B):** cdylib text natives UNIFORMLY produce
>   owned `String` (the FFI always returns a freshly-copied `LoftStr`), so there
>   is no mixed buffer-view/owned hazard — the W trap doesn't apply.
> - **Validated:** the real 2-cdylib consumer (lib/server + lib/web, 7 text
>   natives) compiles `--native`, links, runs to exit 0 on this box (@P389's
>   cross-package link is masked locally by cached deps).  Native suites green
>   serially; `--html` path unexercised (no text-returning cdylib native is
>   browser-compiled — imaging has none).  Regression
>   `pln10_n2_cdylib_text_wrapper_returns_owned_string` (emit-only, not @P389-blocked).

### N2b — cdylib FFI text wrap, interpreter bridge *(the harder half — independent)*
- **Scope:** `extensions.rs:651,894` (`bridge_push_str` / `push_loft_str`).  These
  materialise a foreign `LoftStr` onto the interp stack and need a backing buffer
  for the `Str` ABI — currently `stores.scratch`.
- **Why it's harder than N2a:** the runtime bridge has **no caller-provided
  destination**.  Dynamically-produced value-position text in the interpreter
  must dest-pass (write into a store text record the call site allocates, freed by
  `OpFreeText`), but the chokepoint `wrap_value_text_dest` skips cdylib calls
  (their names aren't in `is_text_dest_native`), so they fall to the scratch
  bridge.  A real fix = give cdylib text natives dest-passing — a new mechanism.
- **Accept:** `scratch.push` −2; cdylib libraries green on the interpreter.
- **Effort/risk:** M / med-high.  **Label:** `area:codegen`

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

### A — `as_text` null-carrying return ✅ **DONE (native)** *(the "blocker" wasn't one)*
> The premise was wrong: **native text-null IS an owned `String` value** — the
> `STRING_NULL` sentinel (`"\0"`), already returned owned by the coroutine text
> path (`STRING_NULL.to_string()`).  So `t_9JsonValue_as_text` returns owned
> `String` (the string for `JString`, `STRING_NULL.to_string()` otherwise),
> preserving `?? x` / `!x`.  This was the **last `codegen_runtime` scratch
> producer → that file is now ZERO `scratch.push`.**  `scratch.push` 31 → 30;
> regression `tests/scripts/195`.  (Interp `n_as_text` + `os_variable` fold into
> the interp-side cleanup with the other fallbacks, at `D`.)

### B — central `Value::Return` / `wrap_result` text wraps (Phase B) *(independent)*
- **Scope (corrected — bigger than first framed):** the 8 `emit.rs`
  `stores.scratch.push((expr).to_string()); Str::new(...)` emissions.  They serve
  **THREE distinct roots**, all "the function body produced an owned `String` but
  the wrapper return type is `Str`, so `Str::new(local)` would dangle":
  1. `no_work_buffer` — @P205 bounded-generic monomorphisation (generics are
     excluded from `text_return` at `definitions.rs:811`, so the concrete copy
     never gets the `RefVar(Text)` work buffer).
  2. `returns_local_text` — @P321e (a work buffer exists but the fn returns a
     *different* local `String`, e.g. a `match` result `.to_string()`'d).
  3. `returns_ncc_block` — @PLAN52 (`??` value-block lowering materialises an
     owned `String` via the `__ncc_*` skip-free pattern).
  To **zero the grep** ALL THREE must stop emitting `scratch.push` — fixing only
  the generic root leaves the emit lines in source (they're string literals — the
  grep counts them even when no program triggers them).
- **The direction + the hazard:** owned-`String` returns (the W/N2 pattern) is the
  clean fix — these roots all produce owned `String`, so returning it directly (no
  `Str::new` wrap, wrapper sig `-> String`) is correct *per return*.  **But** the
  signature is per-FUNCTION while the root-detection is per-`Return`: a fn with
  MIXED returns (one branch a buffer-view `Str::new(work_buf)`, another an owned
  local) can't be a single clean signature — exactly the wrapper/body consistency
  trap `W` hit on wasip2.  **Build a return-shape matrix first** (does any text fn
  mix buffer-view and owned returns?) before committing to the owned-String flip.
- **Accept:** `scratch.push` −8 (emit.rs); `p205_no_str_new_of_local_in_corpus`
  updates to the new shape; all three backends green.
- **Effort/risk:** M-L / med-high (central path, mixed-return hazard).  **Label:** `area:codegen`

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
| **M2 — producers converted** | `N1`✅ `N2a`✅ `A`✅ · `N2b` `I1` `B` | every producer off scratch (fallbacks aside) |
| **M3 — fallbacks gone** | `C`✅ `F` | coverage proof DONE (fallbacks are dead code); `F` deletes them, folded into `D` |
| **M4 — GOAL** | `D` | field deleted |

**Longest chain (critical path):** `W → N1 → N2a`✅ on the native codegen side
(done); the field still waits on `B` (Phase B) + `N2b` (interp bridge) + `I1`
reaching zero, then `C → F → D`.  `A`, `B`, `N2b` are independent and can land any
time before `D`.

**Recommended order (remaining) — by mechanism, not label** (full rationale in
§ the cleanest ordering at the top):
1. **Phase 1 — `I1`-nonnull**: the non-null live producers (proven Build-3 template
   ×N; count-neutral but unlocks D's delete).  Per producer: convert + matrix + commit.
2. **Phase 2 — the null-aware interp primitive → `I1`-null + `N2b`**: the ONE novel
   mechanism (possibly-null owned text on the interp stack), shared by `os_variable`,
   interp `as_text`, and the cdylib bridge.  Reaches **interp scratch = 0** (−2).
3. **Phase 3 — `B`**: central Return emit → owned `String` (matrix-first; mixed-return
   hazard).  Last before D so a thorny `B` doesn't block the interp wins (−8).
4. **Phase 4 — `D` + `F`**: delete the field + the 18 dead bodies (gated on 1–3 = 0).

**Banking the interpreter side is a clean partial:** Phases 1+2 reach `interp
scratch = 0` independently of Phase B.  The field can't delete until Phase B too,
but interpreter scratch traffic reaching zero is most of the Goal-E memory win for
the reference backend.
