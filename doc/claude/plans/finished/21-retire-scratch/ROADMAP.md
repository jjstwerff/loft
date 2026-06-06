---
render_with_liquid: false
---
# @PLN10 — Roadmap to delete `stores.scratch` — HISTORICAL RECORD

**All issues below shipped in PR #277.  This file is a historical record only.**

The dependency-ordered issues that drove completion, with the build evidence each one
built on.  See [`01-destination-passing-design.md`](01-destination-passing-design.md)
for the corrected design doc (also historical).

> ## Final milestone status (all DONE)
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
> | ~~**Phase B** (`emit.rs`)~~ ✅ **DONE** | 0 | central `Value::Return`/`wrap_result` text wraps — Direction A: nwb→owned `String` + buffered local/ncc/ripple→work-buffer write.  Generated native corpus scratch-free (`80cab896`+`c4ea7824`). |
> | ~~**N2b** (`extensions.rs`)~~ ✅ **DONE** | 0 | the interpreter cdylib bridge — **was NOT dead-in-corpus**: the `loft-libs-net/server` cdylib returns text via `LoftStr`, so every HTTP/ws server program hit it.  `Stores::bridge_text_dest` + `n_set_bridge_dest` + `is_cdylib_text_call`/`gen_cdylib_text_dest_call` route it into a work buffer (`3e5312df`+`3c1b51bf`).  **No new opcode.** |
> | ~~**wasm tail** (`native.rs` cfg(wasm))~~ ✅ **DONE** | 0 | `pack_take` + `ws_client_message` — REQUIRED by N2b, not optional: their `loft.toml` `[native.functions]` binding sets `def.native`, so N2b routes them through `n_set_bridge_dest` in wasm too.  Both now `put_owned_text_or_dest` (mirrors `bridge_text_result`) (`8ffe1e3f`).  Validated by construction + non-wasm server equivalence; no in-repo wasm-interp harness (the "ws needs a host" gap). |
> | **D + F** ✅ **DONE (the goal — small-step checklist: [D-execution.md](D-execution.md))** | the dead fallbacks + the field | 🎉 **whole-suite (non-wasm) `=panic` = ZERO (2022/2022)**; the wasm tail is routed too.  EVERY producer is dest-passed — the remaining `scratch.push` are all dead fallbacks (`native.rs` non-`_dest` bodies, `bridge_text_result`'s else, `put_owned_text_or_dest`'s else, the emit.rs Phase-B impossible case).  D deletes them + `Stores::scratch` + the `Scratch` newtype + sentinel + dead ops (`clear_scratch`/`OpClearScratch`).  Residual: the wasm fallback is dead-*by-construction* (no harness), so D removes it on the same evidence as the other dead fallbacks. |
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
> [ISSUE_TRACKING.md § The split](../../../ISSUE_TRACKING.md#the-split--what-lives-where).

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

### N2b — cdylib FFI text wrap, interpreter bridge ✅ **DONE** *(`3e5312df`+`3c1b51bf`)*
- **The reframe that mattered: N2b was NOT dead-in-corpus.**  The `native_pkg`
  test fixture returns only ints, which made it *look* like no text cdylib existed.
  But the real consumer is `loft-libs-net/server` — a cdylib whose request/message
  accessors (`n_tcp_path` / `n_tcp_method` / `n_tcp_body` / `n_ws_message`) return
  `LoftStr`.  Every HTTP/ws server program (multiplayer tests + the markdown viewer)
  routed those through the bridge → `push_loft_str` (`extensions.rs:894`) → scratch,
  and after the I1 RefVar fix this was the **only** remaining live interp scratch.
  So the server lib + its tests **are** the positive control — no fixture needed.
- **Mechanism (no new opcode — op_codes are declaration-order-bound to the
  `OPERATORS` table, so adding one is fragile):**
  - `Stores::bridge_text_dest: Option<DbRef>` — destination for the NEXT cdylib text
    return; set by the `n_set_bridge_dest` native, `take()`n by the bridge.
  - `bridge_text_result` (shared by both bridge sites): dest set → write the
    `LoftStr` into that work-buffer record + push NOTHING; else legacy scratch `Str`
    (now a dead-in-corpus fallback).
  - `is_cdylib_text_call(def)` = `!def.native.is_empty() && returned==Text` —
    disjoint from `is_text_dest_native` (ZERO `default/` decls use `#native`;
    `register_native` is integer-only → no false positives).  Used in the chokepoint
    (`wrap_value_text_dest` / `descend_skip_direct`) + `set_var` (both `Text` and
    return-dep `RefVar(Text)` branches).
  - `gen_cdylib_text_dest_call`: args → dest → `OpStaticCall(n_set_bridge_dest)`
    (pops dest) → `OpStaticCall(cdylib)` (pops args, bridge writes into dest, no
    result pushed).  Frame accounting mirrors `gen_text_dest_call`.
- **Validated:** v3 server under `=panic` no longer trips (was `extensions.rs:894`)
  AND returns correct HTML; multiplayer v2/v3/v5 + viewer_markdown green under
  `=panic`; whole non-wasm suite `=panic` = **2022/2022**.  No regression.
- **Effort/risk:** was M/med-high; the dead-in-corpus de-risk + the no-new-opcode
  design brought it in clean.  **Label:** `area:codegen`

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

### B — central `Value::Return` / `wrap_result` text wraps (Phase B) ✅ **DONE** *(Direction A)*
- **LANDED (`80cab896` nwb→String + `c4ea7824` buffer-write).**  The generated
  native corpus is now **scratch-free** (`scratch.push = 0` across the entire
  `tests/scripts` emit; verified statically AND under `LOFT_SCRATCH_TRIP=panic`).
  Direction A was implemented exactly as the diagnosis predicted — **no buffer
  added, no signature change, no instability** (Direction B's blocker sidestepped):
  - **nwb fns → owned `String`** (`def_returns_owned_text` helper, shared by the
    `mod.rs` wrapper gate + all 4 `emit.rs` return sites: Null-return, If-Return,
    main Return, block-tail).  `Str: Display` makes `(val).to_string()` coerce
    &str / String / Str / buffered-inner-`Str` / nwb-inner-`String` uniformly.
    Generic monomorphs fell out **for free** (they're `nwb`) — no specialisation
    change, the very thing that blocked B.
  - **The ripple, both directions, handled.** `inner_already_str` now EXCLUDES nwb
    inner fns (a `return nwb_helper()` is re-wrapped, not forwarded); a buffered
    outer returning an nwb-inner routes through the buffer-write.
  - **Buffered (`!nwb`) local/ncc/ripple → write the existing work buffer**
    (`return_buffer_name()` → `*var_<buf> = _tmp; Str::new(&*var_<buf>)`), not
    scratch.  The buffer-write path is **live** (1 site: the `155` ncc closure),
    not dead-but-safe.  The scratch emission survives only as an impossible-case
    fallback (`!nwb && no-buffer`).
  - **Positive-controlled** (per the rigor skill): a forced `scratch.push` injected
    into every generated fn PANICS the native child under `=panic` (exit 101,
    `#[track_caller]` names the generated `.rs:line`) — proving the env reaches the
    child and the sentinel fires, so the suite-wide `=panic` silence is *valid*
    evidence of zero, not a dead probe.
  - **All four backends green:** interp, --native (native_scripts + native_dir under
    `=panic`, full native.rs + native_ext + codegen_emitter), --html/wasm32-unknown
    (wasm-html-test 8/8, incl. markdown's nwb `rewrite_link`), wasip2 (`--native-wasm`
    compiles — the W-trap would be E0308 here — + wasmtime runs exit 0).
- **What's left for the field delete (`D`):** N2b (interp cdylib bridge) + I1
  (remaining interp producers) still emit scratch; once those + the suite-wide
  `=panic` gate are zero, `D` deletes the field + the now-dead fallbacks/newtype.
- **Diagnosis history (kept — the two-session matrix that led here):**
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
- **MATRIX BUILT (codegen-only, whole corpus — instrumented the emission sites,
  ran `--native-emit` over `tests/scripts` + `tests/docs`):**
  - 4913 text-return emissions: **3600 `bufview`** (work-buffer view, no scratch),
    **1312 `nwb`** (36 distinct fns, all `wb=false`), **1 `ncc`** (one lambda,
    `wb=true`), **0 `local`** (@P321e is dead in the corpus).
  - **The within-function mixed-return hazard is a PHANTOM** — *no* function mixes
    `bufview` with a scratch kind.  And it's impossible for `nwb`: `nwb` ⟺ no work
    buffer ⟺ no `bufview` return.  So the W-trap (one fn, two return families)
    cannot occur.
- **BUT the matrix surfaced a different hazard — a CROSS-function ripple.**  Flip
  `nwb` fns to `-> String` and a *non*-`nwb` fn (work buffer, `-> Str`) that
  *tail-returns* an `nwb` call (the `inner_already_str` path, `emit.rs:355`) sees
  `String` where it emits `return <call>` expecting `Str` → E0308.  So Direction A
  (owned-`String` flip) is **not** a clean drop-in; it needs the tail-return sites
  bridged (write the inner `String` into the outer's work buffer).
- **Two directions:**
  - **A (owned-`String` flip):** mod.rs `Block && Text && no_work_buffer → String`
    + emit `(val).to_string()`; handle the cross-fn tail-return ripple.  Localized
    to codegen, but the ripple needs care + full 3-rlib (incl. wasip2) validation.
  - **B (work-buffer threading) — RECOMMENDED:** give the `nwb` (generic-monomorph)
    fns a `RefVar(Text)` work buffer (run `text_return` on the concrete copy;
    `definitions.rs:758-771` already does this for I9 interface methods).  Then
    every text return is `bufview` — **no type flips, no ripple, no W-trap.**  The
    `ncc` lambda gets the same treatment (write into its `__work_ret`).  Parser /
    specialisation change, but uniform and ripple-free.  This is the roadmap's
    original framing, now matrix-justified.
- **B PROTOTYPED (then reverted — partial success + one regression):**
  - **The emit reframe works.** Replace `needs_p205_scratch` with `wrap_text &&
    !is_bufview` (is the return the `RefVar(Text)` buffer var?) and emit the
    non-bufview case as `{ let _tmp = (val).to_string(); *var_<wb> = _tmp;
    Str::new(&*var_<wb>) }` (write into the buffer).  Combined with extending the
    P227 force-add from lambdas to **all** text fns (`control.rs` — drop the
    `is_lambda` guard), the corpus went **1313 → 8** `scratch.push` emissions
    (codegen-only count), native + interp smoke green, generated `nwb` fns now take
    `var___work_ret` and write into it.  Everything stays `-> Str` → **no ripple**,
    confirming B's central claim.
  - **The 8 residual:** 6 are generic monomorphs (`text_return` is skipped for
    generic templates → no buffer → scratch fallback; these need the force-add
    threaded through specialisation) + 2 in `05-enums`/`159` (the other two emit
    sites — the `Value::If`-Return at `emit.rs:~318` and the `wrap_result` block-tail
    at `~1452` — still use the old logic; apply the same `!is_bufview` reframe).
  - **The regression ROOT-CAUSED (2nd session, instrumented `rewrite_link`'s
    `text_return` per pass) — B IS BLOCKED.**  The broad force-add **destabilises
    the two-pass signature** of complex multi-return functions.  `rewrite_link`
    (returns `url` arg + two format tails): pass-1 ends at **5 attrs**, pass-2 grows
    to **6 attrs** — the buffer set ACCUMULATES across passes (a pass-dependent
    local-promotion the force-added `__work_ret` perturbs).  The caller (`:763`,
    *before* the def `:821`) resolves the call in pass-2 against the pass-1
    signature (5) while the def finishes pass-2 at 6 → "got 5, need 6".  This is
    NOT a call-site tweak; it's a deep two-pass instability.  (A dep-persist fix was
    tried and did NOT resolve it — the attrs themselves grow, not just the dep.)
- **→ THE DIAGNOSIS INVERTS THE RECOMMENDATION: do Direction A.**  A (owned-`String`
  flip) **adds no buffer → no signature change → no instability** — it sidesteps B's
  blocker entirely.  And the W-trap fear is reduced: the matrix proved `nwb` fns are
  *uniformly* owned (no buffer-view return), so their `-> String` wrapper is
  internally consistent (unlike the blanket flip that broke wasip2 in `W`).
  - **A plan:** (1) `mod.rs:2151` — `def.code==Block && Text && no_work_buffer →
    -> String`; (2) emit the `nwb` return as `(val).to_string()` (owned); (3) handle
    the **cross-fn ripple** — a non-`nwb` fn (has buffer, `-> Str`) that
    tail-returns an `nwb` call (`inner_already_str`, `emit.rs:355`) → write the inner
    `String` into the outer's buffer (rare: only the mixed-outer shape; a pure
    `return helper()` fn is itself `nwb` → also `String` → consistent); (4) `local`/
    `ncc` (have buffers) → write into their buffer (the proven reframe, no
    instability since no new buffer); (5) full 3-rlib (incl. wasip2) + native + wasm.
- **Effort/risk:** M / med — the mechanism is proven (1313→8); the remaining work is
  the call-site-adaptation fix + 2 emit sites + generic monomorphs.  **Label:** `area:codegen`

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

**Recommended order (completed — all shipped) — by mechanism, not label** (full
rationale in § the cleanest ordering at the top):
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
