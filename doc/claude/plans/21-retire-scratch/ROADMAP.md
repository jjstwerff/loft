---
render_with_liquid: false
---
# @PLN10 — Roadmap to delete `stores.scratch`

The clear path from **today** to the **goal (the field deleted; Goal E for
strings — no global text buffer)**, as a set of dependency-ordered issues.

Read [`01-destination-passing-design.md`](01-destination-passing-design.md) first —
it is the design + the build evidence each issue below builds on.

> ## ▶ Next session — start here
> ### 🎉 Milestone — the interpreter generates ZERO scratch traffic
> `LOFT_SCRATCH_TRIP=1` now reads **zero trips across the entire `tests/scripts` +
> `tests/docs` corpus**.  Every native-host text producer dest-passes; no real
> program produces scratch on the interpreter.  This is most of the Goal-E win for
> the reference backend — banked.
>
> **Done (✅):** the interpreter chokepoint; every `codegen_runtime` producer; the
> keystone `W`; the coverage proof `C`; **N2a** (cdylib FFI codegen wrap → owned
> `String`); the **`LOFT_SCRATCH_TRIP` sentinel**; **Phase 1 native-host**
> (`ymd_days_ago`, `store_memory`, `struct_to_json`(+`_pretty`), `i_parse_errors`,
> `parallel_buf`); and **Phase 2 native-host** (`as_text`, `env_variable`).  Each
> sentinel-proven via the positive→negative control pair.  Native + interp green.
>
> ### Phase 2 was a phantom — it collapsed into plain dest-passing
> The roadmap's central Phase-2 claim — *"a dest buffer can't represent null, so
> Phase 2 needs a genuinely-novel null-aware interp primitive"* — was **probe-
> falsified on both backends**.  Text-null is **content-based** (`STRING_NULL` =
> "\0"; `conv_bool_from_text`), so a dest text record carries null by holding the
> "\0" bytes — `?? ` / `!` / `len` / compare / siblings / format read it
> identically to the old sentinel.  So `as_text` dest-passes by writing "\0" for
> null; `os_variable`/`env_variable` is non-null (empty for unset) and dest-passes
> plainly.  **No novel mechanism existed to build.**  (Bonus: per-call dests retire
> the @P354 sibling-aliasing scratch hazard for free.)
>
> ### Surfaced + fixed: a family of null-OUTPUT bugs
> Probing the null model exposed that format interpolation rendered null sentinels
> raw instead of `null`.  Fixed (both backends, one site each in `ops.rs`): text
> "\0" → `null`, float `NaN` → `null` (NaN isn't JSON-standard and `?? `/`!` treat
> it as null).  Integer was already correct.  `inf` is a real non-null value and
> stays `inf`.  Regression `tests/scripts/198`.
>
> ### The metric is the sentinel, not the grep
> The acceptance gate is a **whole-suite `LOFT_SCRATCH_TRIP` zero** (now true for
> the corpus) → the field deletes, its absence the compile-time guard.  Static
> proxy `grep -rn 'scratch\.push(' src/` = **27**, but it over-counts (dead
> fallbacks + the `pre_eval.rs:556` comment + the emitted-code strings) — the
> sentinel is the truth.
>
> ### What's left to delete the field
> | Region | Sites | Nature |
> |---|---|---|
> | **Phase B** (`emit.rs`) | 7 | central `Value::Return`/`wrap_result` text wraps — *emitted* `scratch.push` in generated native code; the mixed-return / W-trap hazard (matrix-gated). |
> | **N2b** (`extensions.rs`) | 2 | the interpreter cdylib bridge (`bridge_push_str`/`push_loft_str`) — write into a dest instead of scratch (now known: plain dest-passing, "\0"/empty for null). |
> | **wasm tail** (`native.rs` cfg(wasm)) | 2 | `pack_take` + `ws_client_message` — non-null; need a *wasm* positive control (`pack_take` testable, `ws` needs a host). |
> | **D + F** | the dead fallbacks + the field | the `native.rs` non-`_dest` bodies are now ALL dead (corpus reads zero); delete them + `Stores::scratch` + the `Scratch` newtype + sentinel + dead ops. |
>
> **Bugs surfaced (orthogonal, filed, NOT blockers):** #272 (native: stateful
> producer in inline `"{x}" != literal`), #273 (native: par-text loop with a
> literal-returning worker → E0061).  Both pre-existing, both with verified
> workarounds.
>
> ### The cleanest ordering (sequence by MECHANISM, not by issue-label)
> Phase B, I1, N2b are mutually **independent** and all gate D, so order is driven
> by risk + subsystem coherence.  Each phase drives a coherent backend region to
> zero; difficulty rises across phases:
>
> 1. **Phase 1 — I1-nonnull** — **NATIVE-HOST COMPLETE.**  `ymd_days_ago`,
>    `store_memory`, `struct_to_json`(+`_pretty`), `i_parse_errors`, `parallel_buf`
>    all dest-pass (14 producers in `is_text_dest_native`, each via the
>    positive→negative control pair).  Audit done — none null on the native-host;
>    the two null producers (`as_text`, `os_variable`) were already Phase 2.
>    **Remaining: the wasm tail** — `pack_take` + `ws_client_message`
>    (`#[cfg(wasm)]`-only, non-null) — deferred to the **wasm-validation pass**
>    (the native-host sentinel can't see them; `ws` needs a live host, so they
>    need a *wasm* positive control rather than shipping unvalidated).  Surfaced
>    orthogonal pre-existing native bugs #272 + #273 (filed, verified workarounds,
>    not blockers — see § below).
> 2. **Phase 2 — null producers → plain dest-passing** ✅ **DONE (native-host).**
>    The "novel primitive" was a phantom (premise probe-falsified): text-null is
>    content-based ("\0"), so a dest carries it.  `as_text` (write "\0" for null)
>    and `env_variable` (non-null, `os_variable` now owns its `String`) dest-pass
>    plainly.  `format.rs` is scratch-free.  **Corpus reads zero.**  N2b (the
>    cdylib bridge) is the same plain dest-passing, just on the FFI bridge — moved
>    to Phase 3.
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
> 4. **Phase 4 — D + F** *(mechanical payoff)*: **gated on a whole-suite
>    `LOFT_SCRATCH_TRIP` run reading zero.**  Delete `Stores::scratch` + the
>    `Scratch` newtype + the sentinel (the field they wrap is gone) + the 16
>    `native.rs` dead bodies + 2 `format.rs` helpers + dead `clear_scratch` /
>    `OpClearScratch` + the `state/codegen.rs:329` emission (and the
>    `pre_eval.rs:556` comment).  The loft *def* stays for the IR `Call`; a
>    `library_names` miss then loudly catches any residual emit.  Gated on 1–3
>    reaching zero.
>
> **The single insight (revised by the build):** the design feared a novel
> null-aware primitive; **probing the load-bearing claim dissolved it** — text-null
> is content-based, so null rides plain dest-passing.  The only genuinely-hard
> piece left is **Phase B's mixed-return / W-trap hazard** (matrix-gated); N2b is
> now plain dest-passing on the cdylib bridge; Phase 4 (D) is the mechanical
> payoff, gated on the sentinel reading zero.
>
> 🔧 **The sentinel — your live-vs-dead oracle (use it every batch):**
> ```
> LOFT_SCRATCH_TRIP=1 ./target/release/loft --interpret <file>   # warn: print each live push file:line
> LOFT_SCRATCH_TRIP=panic ./target/release/loft --interpret <file>  # backtrace the first hit
> # enumerate the whole corpus (the "what's still live" map):
> for f in tests/scripts/*.loft tests/docs/*.loft; do \
>   LOFT_SCRATCH_TRIP=1 ./target/release/loft --interpret "$f" 2>&1 | grep LOFT_SCRATCH_TRIP; \
> done | sed 's/.*@ //' | sort | uniq -c | sort -rn
> ```
> A converted producer must produce **zero trips** in its matrix probe; a
> whole-suite zero is the `D` acceptance gate.  (Interpreter-side only — native
> generated programs don't touch `Stores::scratch`.)
>
> ⚠ **Silence needs a positive control** (engineering-rigor § the usage sentinel):
> a zero-trip probe reads the same whether the producer is converted *or the probe
> never exercised it*.  So pair it: confirm the sentinel CAN fire for that value-
> position shape first — an unconverted producer in the same positions trips (the
> batch-1 control was `as_text`), and the IR dump shows the chokepoint actually
> wrapped your producer's calls (`__work_N = your_producer(...)`).  The
> whole-suite `D` gate gets this free (if anything used it, you'd see it); a small
> per-batch probe does NOT.
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

**Goal:** delete `Stores::scratch` (`src/database/mod.rs`) + its `Scratch` newtype
and `LOFT_SCRATCH_TRIP` sentinel, the dead `clear_scratch` (`fill.rs`), the no-op
`OpClearScratch` (`default/02_files.loft`), and the dead per-`Line` emission
(`state/codegen.rs:329`).

**Acceptance (runtime, via the sentinel — supersedes the grep):** a whole-suite
run under `LOFT_SCRATCH_TRIP=1` reports **zero trips** → nothing depends on
scratch → the field + newtype delete, and their absence is the compile-time
guard.  (The bare `grep scratch.push` is no longer the metric: the sentinel's own
doc/output strings pollute it, and it always over-counted dead vs live.  Static
proxy while converting: `grep -rn 'scratch\.push(' src/`.)  `tests/scripts/192`–`196`
stay green on **all three backends** (interpreter / native / wasm) throughout.

**Done so far:** the interpreter synth-temp chokepoint, all native `codegen_runtime`
cell-ABI producers, the keystone `W`/`N1`, the coverage proof `C`, `A` (native
`as_text` null), `N2a` (cdylib FFI codegen wrap), the `LOFT_SCRATCH_TRIP`
**sentinel**, and **Phase 1 batch 1** (`ymd_days_ago` + `store_memory`).  27 real
`scratch.push(` statements remain; the sentinel reads **4 live sites** across the
corpus (`as_text`, `parallel_buf`, `os_variable`, `i_parse_errors`).

---

## The dependency graph

```
   W (keystone)✅ ─► N1✅ ─► N2a✅ ─────────────────────────┐
                                                            ▼
   A (null)✅ ──────────────────────────────────► C✅ ─► F ─► D  (GOAL)
                                                    ▲
   ── remaining (each gates D, mutually independent) ──
   Phase 1: I1-nonnull (batch 1 ✅ ymd/store_memory)┤   (count-neutral; makes
   Phase 2: null primitive → I1-null + N2b ─────────┤    D's delete safe / −2)
   Phase 3: B (central Return emit) ────────────────┘   (−7)

   tool: LOFT_SCRATCH_TRIP sentinel ✅ — runtime live-vs-dead oracle + D's gate
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

### I1 — remaining interpreter producers → dest-passing *(= Phase 1; native half done via #rust/codegen_runtime)*
- **Scope:** per producer, the proven Build-3 pattern — a `_dest` variant +
  `is_text_dest_native` entry (interp).  The native half is already scratch-free
  (the `#rust` template inlines an owned `String`, or the `codegen_runtime` body
  returns one), so I1 is **interp-only**.  **Audit each for null-safety first** —
  null-carrying producers can't dest-pass (a buffer can't represent null) and
  route to **Phase 2** instead.
- **Status (sentinel-classified):**
  - ✅ **batch 1 DONE** — `n_ymd_days_ago`, `n_store_memory` (non-null, zero-trip).
  - ✅ **batch 2 DONE** — `n_struct_to_json`(+`_pretty`), `i_parse_errors`
    (non-null; full positive→negative control pair).  Surfaced orthogonal native
    bug #272 (stateful producer in inline `"{x}" != literal`) — filed, not a
    regression of this conversion.
  - ✅ **parallel_buf DONE** — `n_parallel_buf_get_text` (non-null; interp
    control-pair).  Native par-text is blocked by #273 (separate, in the worker
    closure — not this read-side conversion), so native runtime-validation waits
    on #273; the conversion is native-transparent by construction.
    **→ native-host Phase 1 COMPLETE.**
  - **wasm tail (deferred to the wasm-validation pass):** `n_pack_take`,
    `n_ws_client_message` — `#[cfg(wasm)]`-only, non-null.  Need a *wasm* positive
    control (`pack_take` wasm-testable; `ws` needs a host).
  - **→ Phase 2 (null, sentinel-located):** `os_variable` (`format.rs:333`),
    interp `n_as_text` (`native.rs`).
- **Accept:** each producer **zero-trip** under `LOFT_SCRATCH_TRIP` in value
  position (matrix), both backends.
- **Effort/risk:** S-M / low (mechanical × N; the proven template).  **Label:** `area:codegen`

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
| **M2 — producers converted** | `N1`✅ `N2a`✅ `A`✅ `I1`(native-host)✅ · `I1`-wasm-tail `N2b` `B` | native-host producers off scratch (sentinel reads only Phase-2 null) |
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
