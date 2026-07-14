<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN104 — text-return promotion corpus (P1, codegen gate step 1)

The working-vs-broken bytecode corpus for the interpreter owned-text leak
(loft-lang/loft#568, bisected to #551). One function per text-return **delivery
path**, so a fix (P3) can prove it changed only the leaking paths and left the
clean ones byte-identical.

## The paths + measured status (main-tip, this branch's base)

Per-path leak = `Direct leak` count from the ASan interpreter build
(`scripts/asan_leak_scan.sh`); return type from `loft introspect corpus.loft`.

| fn | classification | leak? | BROKEN return | notes |
|---|---|---|---|---|
| `ret_fnref` | `Owned:FnRefCall` (pass-2 only) | **LEAK** | `text` (bare) | owned by value → interpreter orphan |
| `ret_index` | `Owned:ViewOfLocal` (pass-2 only) | **LEAK** | `text["v"]` | view of a local freed at scope exit |
| `ret_borrow` | `Borrow(Argument)` | clean | `text["s"]` | borrows the caller's arg — **must NOT promote** |
| `ret_local` | `Owned:BuiltLocal` (pass-stable) | clean | `text["r"]` | already delivered via the local `r` |
| `ret_interp` | `Owned:BuiltLocal` (pass-stable) | clean | `text["__work_1"]` | already retbuf-delivered |

Positive controls = the two LEAK rows; negative controls = the three clean rows
(the harness must keep them clean, and must never promote `ret_borrow`).

## The spec (what P3 must emit)

The two leaking returns must gain a delivery dep like the clean owned rows — a
`__tret`/retbuf so the caller allocates + frees the buffer — WITHOUT touching
`ret_borrow` (arg-borrow, no buffer — the @P273/@P387 reversion) or changing the
already-correct `ret_local`/`ret_interp` emission. The promotion decision keys off
`use_analysis::return_ownership` (the backend-shared verdict `--show-ownership`
renders), not the pass-unstable `classify_text_return` — see the plan.

## Reproduce

```sh
# per-path leak (needs an ASan loft + llvm-symbolizer for the leak:ir_read suppression)
ABIN=target/x86_64-unknown-linux-gnu/release/loft \
  LSAN_OPTIONS=suppressions=.github/lsan_suppressions.txt ASAN_OPTIONS=detect_leaks=1 \
  "$ABIN" --interpret <one-path>.loft

# BROKEN IR (this file's baseline)
loft introspect corpus.loft > broken.ir

# ownership overlay (backend-shared; the fix flips ret_fnref/ret_index to buffer-delivery)
loft introspect --show-ownership corpus.loft
```

`broken.ir` is the captured baseline. `good.ir` (P3) is the post-fix capture; the
diff must be confined to `ret_fnref` + `ret_index` (+ their call sites in `main`).

## P2 result — the oracle pass + the two-class partition

`report_tret_promotions` (env `LOFT_TRET_REPORT`, `parser/control.rs`, run after
`mod.rs:1139`) flags a user text-returning def for promotion iff it has **no** hidden
`&text` retbuf AND its return is backed **frame-locally**:

- `return_ownership == Owned` (a fresh local store — `ret_fnref`), OR
- `Borrowed{base}` / `Join{base}` where `base` is **not an argument** (a view of a
  local — `ret_index`, `base == u16::MAX` = names no visible param → a local view).

A `Borrowed` of an **argument** (`ret_borrow`, `base = 0 = s`) is skipped — the caller
owns it and it outlives the frame. Verified: flags `ret_fnref` + `ret_index`, skips
`ret_borrow`/`ret_local`/`ret_interp`.

**On the real nightly leakers it partitions the class:**

| class | files | P2 |
|---|---|---|
| text-return-tail (this fix) | 387, 85-poison-return-tail-uaf, 85-ncc-container-text-return, 552, 553, 557 | flags |
| **match field-projection (SEPARATE)** | 35n-field-projection, 35p-iterator-match | flags 0 |

`35n`/`35p` leak despite `words` already carrying its `__work_1` retbuf — the orphan is
the match's extracted `w: vector<text>` temporary, not the text return. That is a
distinct bug outside #568's scope (own investigation). The oracle pass making this
boundary visible is P2's main deliverable.

## P3 status — third-pass promotion WORKS structurally; one dep bug remains

`force_tret` (`parser/mod.rs`) + the gate `|| self.force_tret.contains(&self.context)`
(`control.rs`) + a **third `parse_file` pass** (opt-in `LOFT_TRET_FIX`, `mod.rs` after
`:1139`) implement the forward-ref-safe promotion: P2 marks the flagged defs, then the
third pass promotes them with the retbuf attr present from the start, so every caller
re-lowers with the buffer.

**Verified working (corpus + min.loft, both backends):**
- flagged defs gain the `___tret` retbuf; output correct on interp AND native;
- callers allocate + pass + free the buffer (balanced: 4 allocs / 4 frees in corpus main);
- the promoted `ret_fnref` body is byte-identical to the verified-clean workaround
  (`r = f(x); r`) EXCEPT the return-type dep.

**The one remaining bug (blocks leak-free):** the promoted `run_t` return type is
`text["__work_1"]` (the fn-ref call's INTERMEDIATE buffer) where it must be
`text["___tret_1"]` (the retbuf) — cf. the clean workaround's `text["r"]`. That wrong
dep mis-tracks the delivery, so the `append_text` result still orphans (1 real
interp leak persists; native RAII-clean). Fix is in the promotion's return-dep
computation (`text_return` / the fn-ref-buffer interaction), NOT the machinery.

### P3 root cause pinned (2026-07-13) — var-order dep resolution

The full IR diff (`min.loft` +LOFT_TRET_FIX vs the verified-clean `r=f(x);r`
workaround) shows the promoted `run_t` is FUNCTIONALLY identical — same native Rust,
same interp bytecode — with one real divergence in the ownership resolution:

```
--show-ownership:  fix → text["__work_1"]     workaround → text["RB"(=r)]
var order:         fix: __work_1@2, ___tret_1@3   workaround: r@2, __work_1@3
```

`do_tret_bind` mints `___tret_1` (the retbuf) AFTER the fn-ref call's intermediate
buffer `__work_1`, so it gets a higher var index. `use_analysis::return_ownership`
resolves the return dep to the FIRST hidden `&text`/text buffer — `__work_1`, which is
a LOCAL freed inside `run_t` (`FreeText(__work_1)`), not the retbuf. That mis-resolved
dep makes the interpreter deliver/free the wrong buffer, so the `append_text` result
orphans (1 interp leak; native RAII-clean).

**Fix direction:** exclude the fn-ref's freed intermediate buffer from return-dep
candidacy (it is `FreeText`'d in-body, so not a delivery), OR mint `___tret_1` such
that it resolves first. NOT the third-pass machinery, which is correct. Next increment.

### CORRECTION — the return-dep verdict was NOT the root cause

Attempted the return-dep fix: in `use_analysis::classify`, an ARGUMENT var →
`Borrowed{base=self}` regardless of any `Set` RHS (so a retbuf arg isn't followed
back to a freed local). It DID correct the verdict — `--show-ownership` then reports
`run_t -> Borrowed(base=___tret_1)` (the retbuf, not `__work_1`) — but the runtime
`append_text` leak **persisted** (min.loft + corpus, both still 1). So the mis-resolved
verdict was a *symptom*, not the cause. Reverted (a non-gated change to the shared
oracle that doesn't fix the leak isn't worth the risk).

**Net:** the promoted `run_t` is functionally equivalent to the verified-clean
`r=f(x);r` workaround (same native Rust, same interp opcode stream, and — after the
above — the same ownership verdict), yet leaks where the workaround doesn't. The
remaining difference is the VAR ORDER / slot assignment (`__work_1`@2 vs `___tret_1`@3),
and the leak survives every static fix tried. This needs a **runtime** diagnostic
(live debugger / store trace to catch the exact un-freed allocation at execution
time), not more static IR analysis — the next increment's starting point.

### RECHECK (2026-07-13) — the "premise in doubt" was WRONG; P1–P3 proven not to fix it

The prior "runtime-trace session" concluded `min.loft` doesn't leak and the direction
is a red herring. **Both halves were wrong**, and the recheck reverses them with
reliable measurements.

**Measurement method that finally works locally** (no `llvm-symbolizer` needed): an
ASan build with `-Cforce-frame-pointers=yes` (deep AND fast stacks) + slow unwind
(`ASAN_OPTIONS=fast_unwind_on_malloc=0`) + a Python post-filter (`realleak.py`) that
symbolizes with `addr2line` and drops `ir_read` frames — the interner suppression the
missing runtime symbolizer couldn't apply. Runs are slow (the third pass amplifies
`ir_read` to ~300); run each in the background so LSan reports instead of timing out.

Three-way, reliable (each run finished, ir_read-suppressed):

| program | leak |
|---|---|
| baseline `min.loft` (bare `f(x)` tail) | **1** `fill::append_text` |
| `min.loft` + `LOFT_TRET_FIX=1` (P3) | **1** — identical to baseline |
| `min_wa.loft` (`r = f(x); r`) | **0** — clean |

So `min.loft` **is a faithful repro** (baseline leaks). The earlier "all freed" trace
was instrumenting the wrong function — it hooked `State::append_text` where the ASan
leak's inlined site is the SAME method but the trace logic missed the call; a corrected
per-pointer trace (`LOFT_PTR_TRACE`, reverted) shows the append buffer orphans.

**The pointer trace pins it** — two `"v1"` allocations exist; the leak is the appended
one, never freed:

- baseline/fix: `run_t` runs `AppendText` → allocates `"v1"` at ptr **A** (never freed);
  a copy **B** is freed. **A leaks.** Fix trace is byte-identical to baseline.
- workaround: **no append at all** — the buffer is *moved/renamed*; both copies freed.

**The definitive bytecode diff (`run_t`, fix vs workaround) — the real fault:**

| | signature | delivery op |
|---|---|---|
| workaround (clean) | `(f, x, r:&text) -> text["r"]` | `AppendStackText(r)` — into the caller's buffer via the ref |
| fix (leaks) | `(f, x) -> text` — **no retbuf** | `AppendText(__ret_1)` — into an owned local, returned by value |

The P3 retbuf **is present in the IR** (`___tret_1:&text -> text["___tret_1"]`) but is
**dropped during compilation** — the compiled `run_t` has signature `-> text` with an
owned `__ret_1` + `AppendText`, exactly the leaking owned-text-by-value return. That is
why the runtime never changed: the promotion never reaches codegen.

**Root blocker — pass timing.** The workaround's `r` is promoted on **pass 1**, so its
`&text` retbuf param is baked into the signature `compile.rs` reads. P3's `__tret` is
promoted on the **third pass** (the `use_analysis` oracle can't classify a fn-ref /
local-index tail until after pass 2), *after* the signature was finalized on pass 2 —
so compile emits the pass-2 signature (no retbuf). Promoting on pass 2 instead violates
the H5 two-pass attribute-count contract (`assert_pass2_def_attr_stable`) — that is
exactly the crash #551 gated off, reintroducing this leak.

**The IR-level fixes tried this session (kept, gated on `LOFT_TRET_FIX`):** block-dep
then frame-dep preservation in `block_result`'s `Type::Text` arm — they make the IR
block type `text["___tret_1"]` (matching the workaround's `text["r"]`), but the leak
persists because the fault is downstream in codegen, not the IR block dep. Necessary
housekeeping, insufficient alone.

**The direction that CAN work (next increment):** make the fn-ref-call / local-index
text-return promotion land in the finalized signature `compile.rs` reads — options:
(a) classify these tails on **pass 1** (needs pass-1 lowering to recognise the shape),
or (b) let the third pass **re-finalize the def signature** so `compile.rs` emits the
retbuf `AppendStackText` path (honor the post-pass-2 retbuf) instead of the owned
`__ret_1` `AppendText` fallback. The target bytecode is captured above — the workaround's
`run_t` is the byte-exact goal.
