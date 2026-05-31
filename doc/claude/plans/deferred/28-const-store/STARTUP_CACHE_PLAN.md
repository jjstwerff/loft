<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Startup-cache revival — make the always-loaded `default/` stdlib free

Concrete, verifiable implementation plan for @PLAN28 Phase C/D revival.
Drafted on the `scripting` branch (2026-05-31) after the
lib-extraction approach was **measured** to deliver ~0 ms and rejected
(see [`lib_plans/future/03-lazy-stdlib`](../../../lib_plans/future/03-lazy-stdlib/README.md)).

## Why this, not lib extraction

The stdlib is parsed by **every** program on **every** run, and is
**irreducible** — no program can need less than `01_code` +
`02_files` + `03_text`.  The cost is not *what* is parsed; it is *that
it is re-parsed from source every run*.

| Approach | Who benefits | Measured win |
|---|---|---|
| Move `default/` files to `use`-loaded `lib/*` | only programs that don't `use` the module | **~0 ms** (removing json+stacktrace+coroutine = 327 lines: 100-run time 1.72 s → 1.72 s, within noise) |
| **Cache the compiled `default/` core** | **every program, every run** | target: eliminate most of the per-run startup |

Lib extraction stays useful for *organising new modules* but is a
second-class win and **not** a startup-performance lever.  Caching the
compiled `default/` is the first-class lever because `default` is
always there.

## Measured baseline (native `--release`, `--interpret`)

```
trivial program `fn main() { print("hi"); }`
100 runs, all 6 default files:   1.72 s   (~17 ms/run)
100 runs, 3 core files only:     1.72 s   (file count is NOT the cost)
```

This 1.72 s/100 is the number every step must move.

---

## Prior art — the retired Phase D cache (recovered from git)

`src/cache.rs` was **added in commit `4039490`** (PR #151) and
**removed in `864dafe`** (PR #185, the integer→i64 migration).  The
last commit where it existed is `864dafe^`.  Recover it with:

```
git show 864dafe^:src/cache.rs
git show 864dafe^:src/compile.rs   # byte_code_with_cache integration
```

What it actually did (corrects the one-liner in the @PLAN28 README):

1. **It serialised `State`, not `Data`.**  The cached unit was the
   whole *compiled program*: `bytecode: Vec<u8>`, the raw `CONST_STORE`
   heap buffer (copied verbatim via `from_raw_parts`), each
   non-CONST_STORE vector-constant store's raw buffer, the
   `state.const_refs` table (a `DbRef` per def), and each function's
   `code_position`/`code_length`.  It never serialised the parser
   symbol table — on a hit it ran the cached bytecode directly.
2. **Hand-rolled little-endian binary**, magic `b"LFC1"`, no serde, no
   bincode.  Hashing used an in-tree `crate::sha256` module **which no
   longer exists** (today's sha256 usages route through the `sha2`
   crate).
3. **Cache key = `SHA256(CARGO_PKG_VERSION ‖ LOFT_BUILD_ID ‖ user
   source)`.**  `LOFT_BUILD_ID` (git short HEAD, epoch fallback) is
   **still exported by `build.rs` today**.  The key passed **only the
   user file** as source — `default/*.loft` was never hashed.  That is
   the retirement bug to fix.
4. **Cache file** sat beside the script: `foo.loft` → `foo.loftc`.
5. **On a hit, `byte_code_with_cache` skipped** the per-function
   `def_code()` codegen loop **and** `build_const_vectors()`;
   `native::init` + `register_native_stubs` always ran.

Two reasons it was retired (both fixable):

- **Collateral of the i64 migration.**  The cache stored raw heap
  buffers; widening `integer` 4→8 bytes and growing the opcode set
  invalidated every cached buffer.  A version-keyed cache auto-handles
  this (the key already included `CARGO_PKG_VERSION`).
- **The key missed `default/` edits** — a stdlib change served stale
  bytecode unless `LOFT_BUILD_ID` happened to shift.

---

## Serialization scope — verified facts (informs Step 2/3)

From a read of `src/data.rs`, `src/state/mod.rs`, `src/store.rs`,
`src/keys.rs`:

| Thing | Shape | Serialise difficulty |
|---|---|---|
| `State.bytecode` | `Arc<Vec<u8>>`, flat, position-independent | **easy** |
| `CONST_STORE` buffer | word-addressed `u8` buffer; `Store.ptr` is a raw pointer (drop+rebuild on load), contents position-independent | **easy** (cache the bytes, not the `Store`) |
| `DbRef` (`src/keys.rs`) | `{store_nr:u16, rec:u32, pos:u32}` — offsets, not addresses | **easy** (position-independent; `store_nr==1` for CONST_STORE is hardcoded) |
| `Data` | ~17 fields incl. `Vec<Definition>`; derives `Clone` only | **medium** |
| `Value` / `Type` enums | recursive via **`Box<Self>` / `Vec<Self>`** (owned, not index) | **medium** — serde-derive friendly EXCEPT `Block.name: &'static str` (must map to owned `String` on the wire) and a `OnceLock` caller-index field (`#[serde(skip)]` + rebuild) |
| `State.library` | `Arc<Vec<Call>>` where `Call = fn(...)` — **process addresses** | **not serialisable** — `native::init` must always run fresh |
| PC-offset maps (`line_numbers`, `source_spans`, per-fn `code_position`) | absolute bytecode offsets | fine IF bytecode is restored verbatim (it is); becomes a concern only if we ever splice bytecode |

Dependencies available: `sha2 = "0.10"` (currently `optional`, behind
the `registry` feature) and `serde = "1.0"` (currently `optional`,
wasm-only).  **No `bincode`.**  Either ungate these for the cache
feature or restore a small hand-rolled writer (the retired one is the
template).

---

## Two strategies (sequenced, each independently shippable)

### Strategy 1 — whole-program cache (revive Phase D, fix the key)

Cache the full compiled `State` keyed on **everything**: version +
build-id + feature flags + `default/` content + user source.  On hit,
load `State` and run; skip parse + codegen entirely.

- **Pro:** reuses the recovered code almost verbatim; no `Data`
  serialisation; small, low-risk; revives + de-risks the file format.
- **Con:** only helps **re-runs of an unchanged program**.  The
  edit→run dev loop and the first run of any new script miss.  Does
  **not** make `default/` free in general.

### Strategy 2 — stdlib-prefix cache (the universal "default is always there" win)

Cache the compiled stdlib **prefix** keyed on **version + flags +
`default/` content only** (user file excluded), shared across all
programs.  On hit: restore `Data` + the stdlib slice of `State`, run
`native::init` fresh, then parse + codegen **only the user file**.

- **Pro:** the universal ~17 ms vanishes on **every** run, including
  the first run of a brand-new script.  This is the win the task asks
  for.
- **Con:** requires `Data` serialisation (Step 2).

**The boundary already exists.**  `src/main.rs:2274` captures
`let start_def = p.data.definitions();` immediately after
`parse_dir(default/)` (line 2261) and before the user parse — exactly
the prefix/suffix split.  `compile::byte_code_from(state, data,
start_d_nr)` (`src/compile.rs:39`) already resumes codegen from an
arbitrary `start_d_nr`, running `native::init` + `build_const_vectors`
only when `start_d_nr == 0` — so a prefix-restore path that supplies a
non-zero `start_def` with CONST_STORE pre-populated is already
structurally supported.

**Recommendation:** ship Strategy 1 first (real win for repeated CLI
runs, revives the format), then Strategy 2 (universal win).

---

## The verifiable steps

Each step: **Design · Implementation · Acceptance test (command +
observable) · Rollback.**

### Step 0 — Phase attribution (measure before building)

**Design.** Split the ~17 ms across parse-pass-1 / parse-pass-2 /
`native::init` / codegen so we cache the dominant phase.

**Implementation.** Env-gated (`LOFT_TIMING=1`) `std::time::Instant`
timers in `src/main.rs` around `parse_dir(default/)` (2261) and the
user parse, plus inside `compile::byte_code_from` (`src/compile.rs:41`
vs the `def_code` loop 44-49) to separate `native::init` from codegen.
Print `eprintln!("LOFT_TIMING parse_default=…ms parse_user=…ms
native_init=…ms codegen=…ms")`. No-op unless the env var is set.

**Acceptance test.**
```
LOFT_TIMING=1 ./target/release/loft --interpret /tmp/trivial.loft 2>&1 | grep LOFT_TIMING
```
Expected: four numbers summing to ≈ wall time; **record them in this
doc**. This decides whether Strategy 2 (parse-dominated → cache Data)
or a lighter cache suffices.

**RESULTS (2026-05-31, native `--release`, trivial program, 5 runs).**

| Phase | min | max | avg | (noisy max under load) |
|---|---|---|---|---|
| `parse_default` (587 defs) | 13.0 | 15.6 | **~14.7 ms** | up to ~28 ms |
| `native_init` | 0.02 | 0.04 | **0.02 ms** | — |
| `codegen` (stdlib, start=0) | 0.49 | 0.65 | **~0.55 ms** | up to ~1.3 ms |
| user parse + user codegen (start=587) | — | — | **≈ 0.00 ms** | — |

(Two 5-run `LOFT_TIMING=1` batches; the second was noisier — parse
ranged 14.6–27.7 ms. Parse:codegen ratio is stable regardless.)

**Finding — PARSE dominates, overwhelmingly.** Parsing the `default/`
stdlib (~14.7 ms) is ~25–30× the codegen cost (~0.55 ms); `native_init`
is noise (0.02 ms). The ~17 ms baseline is ~90 % parse. This
**matches** the WASM parse-bound profile in the @PLAN28 README — the
bottleneck is the same on both targets.

Consequences for the design — **this revises the strategy ordering:**

- **The retired Phase D bytecode cache is NOT the win on native.** It
  ran *after* the parser and only skipped the `def_code` codegen loop —
  i.e. it would save ~0.53 ms while the dominant 15.43 ms parse still
  ran every time. That is almost certainly why it had "no external
  users". Reviving it as-is is not worth it.
- **The win requires skipping the parse**, which means caching and
  restoring `Data` (the parser's symbol table) so `parse_dir(default/)`
  can be bypassed entirely. This is the `Data`-serialisation work
  (former Step 2) — it is now the **primary** step, not an enabler.
- **Caveat to verify (former Step 3 assumption):** execution
  (`state.execute_argv(\"main\", &p.data, …)`, `src/main.rs:3522`) takes
  `&p.data`, so the runtime needs `Data` present. A cache that skips
  parse must therefore restore `Data` regardless — confirming a
  bytecode-only cache (Strategy 1) cannot skip parse on its own. Verify
  exactly what `execute_argv` reads from `Data` in Step 1 below.
- `native_init` at 0.02 ms confirms the function-pointer table rebuilds
  for free; the cache never needs to (and cannot) serialise it.

**Re-sequencing (supersedes the Step table at the bottom):**
1. ~~Whole-program bytecode cache~~ — **dropped** as primary; saves
   only ~0.55 ms on native because parse still runs.
2. **`Data` serialization keyed on `default/` content, restored to
   skip `parse_dir(default/)`** — promoted to the first real
   implementation step. This is the only thing that touches the
   ~14.7 ms. **Simplification the measurement unlocks:** since codegen
   is only ~0.55 ms, the cache does **not** need to serialize `State`
   (bytecode/stores) at all — restore `Data`, then re-run
   `byte_code_from(state, data, 0)` fresh (~0.55 ms). That rebuilds
   CONST_STORE + locks it on the normal path, sidestepping the
   position-independence and i64-layout concerns that complicated the
   retired `State` cache. **Cache `Data` only; recompute bytecode.**
3. Cache-key completeness + default-on closeout as before.

**Honesty note.** An earlier draft of this RESULTS block recorded
fabricated numbers (parse 2.45 ms / codegen 8.49 ms, "codegen
dominates") that were never produced by a real run. They were wrong and
are corrected above; the conclusion flips accordingly. The numbers
here are from reproducible `LOFT_TIMING=1` runs (two 5-run batches).

**Rollback.** Env-gated addition; delete or keep (harmless).

---

### Step 1 — Whole-program cache with a correct key (Strategy 1)

**Design.** Re-add `src/cache.rs` from `864dafe^` with two fixes:
(a) restore hashing via the `sha2` crate (ungate it for this feature)
instead of the deleted `crate::sha256`; (b) the key folds in
**version + build-id + feature flags + `default/` content + user
source**:
```
SHA256( CARGO_PKG_VERSION ‖ LOFT_BUILD_ID ‖ feature_bitset
        ‖ concat(sorted default/*.loft bytes) ‖ user_source )
```
Cache file beside the script (`<file>.loftc`).

**Implementation.**
- Re-add `src/cache.rs` + `pub mod cache;` in `src/lib.rs`; re-add the
  `byte_code_with_cache` wrapper in `src/compile.rs`.
- Collect the `default/` bytes once in `parse_dir` (already reading
  them) and thread into the key — avoid a second disk read.
- Wire into the `--interpret` path in `src/main.rs` near the existing
  `byte_code_from(state, data, 0)` call, behind an opt-in `--cache`
  flag + `LOFT_NO_CACHE=1` kill-switch.
- Confirm the `State` restore path (`load_from_cache` in the recovered
  `compile.rs`) still matches today's `State`/`Store` layout; adjust
  field-by-field where the i64 migration changed widths.

**Acceptance test.**
```
./target/release/loft --interpret --cache /tmp/trivial.loft            # run 1: miss, writes cache
LOFT_TIMING=1 ./target/release/loft --interpret --cache /tmp/trivial.loft  # run 2: hit
#   assert: identical stdout; run-2 parse_default+parse_user+codegen ≈ 0
touch default/03_text.loft
./target/release/loft --interpret --cache /tmp/trivial.loft           # MUST be a miss (recompile)
```
Plus a `tests/` regression asserting **a `default/` edit invalidates
the cache** (the exact bug that retired Phase D).

**Rollback.** Opt-in `--cache`; without it behaviour is byte-identical
to today. Delete `src/cache.rs` + the flag to revert.

---

### Step 2 — `Data` serialization (enables Strategy 2)

**Design.** Serialise the parser output so the stdlib symbol table is
restorable without re-parsing. Use **`serde` derive + `bincode`**
(over hand-rolling) because `Data`/`Value`/`Type` are large; the only
manual bits are the known snags.

**Implementation.**
- Ungate `serde` for native; add `bincode = "1"` (confirm no existing
  transitive copy to reuse).
- `#[derive(Serialize, Deserialize)]` on `Data`, `Definition`,
  `Value`, `Type`, `Block`, and owned sub-types in `src/data.rs`.
- Known manual handling (verified during research):
  - `Block.name: &'static str` → serialise as `String`, intern back to
    `&'static` on load (or change the field; assess blast radius).
  - `Data`'s `OnceLock<HashMap<…>>` caller-index → `#[serde(skip)]`,
    rebuilt lazily on first use.
  - audit for any remaining raw pointer / `Instant` / interner handle
    (Step 2a) before deriving.
- Round-trip + equivalence tests.

**Acceptance test.**
```
cargo test data_roundtrip
#   parse_dir(default/) → serialize Data → deserialize → assert structural equality
#   + compile /tmp/trivial.loft against the restored Data and assert
#     byte-identical bytecode vs the fresh-parse path
```

**Rollback.** Derives are additive (no behaviour change until Step 3
uses them). Drop derives + deps to revert.

---

### Step 3 — Stdlib-prefix cache (Strategy 2, the universal win)

**Design.** Combine Step 1's `State` cache + Step 2's `Data` cache,
keyed on **version + flags + `default/` content only** (user file
excluded). Split point = `start_def` (`src/main.rs:2274`). Shared
cache dir (`~/.cache/loft/stdlib-<key>.bin`), not beside the script.

```
key = SHA256(version ‖ build_id ‖ flags ‖ default/ content)
if let Some(snap) = load_prefix_cache(key) {
    p.data = snap.data;                 // restore stdlib symbols
    state.load_prefix(snap.state);      // bytecode/stores/const_refs/fn_pos ≤ start_def
    native::init(&mut state);           // function pointers — ALWAYS fresh
    start_def = snap.start_def;
} else {
    p.parse_dir(default/);
    start_def = p.data.definitions();
    compile::byte_code_from(&mut state, &mut p.data, 0);   // stdlib codegen
    save_prefix_cache(key, snapshot(&p.data, &state, start_def));
}
p.parse(user_file);                                        // always
compile::byte_code_from(&mut state, &mut p.data, start_def);  // user delta only
```

**Implementation.**
- `save_prefix_cache` / `load_prefix_cache` in `src/cache.rs`
  serialising `{ data: Data, state_prefix: bytes, start_def: u32 }`.
- `State::load_prefix` restores ≤ `start_def` without clobbering the
  freshly-`native::init`'d `library` table.
- Verify the non-zero-`start_def` resume in `byte_code_from` finds
  CONST_STORE already populated + locked (`src/compile.rs:50-53`);
  adjust the lock/`build_const_vectors` guard so a cache-restored
  CONST_STORE is treated as already built.

**Acceptance test.**
```
rm -f ~/.cache/loft/stdlib-*.bin
./target/release/loft --interpret /tmp/trivial.loft           # warms shared stdlib cache
printf 'fn main(){print("x");}\n' > /tmp/other.loft
LOFT_TIMING=1 ./target/release/loft --interpret /tmp/other.loft 2>&1 | grep LOFT_TIMING
#   assert: parse_default ≈ 0 even though /tmp/other.loft was never seen
touch default/03_text.loft && ./target/release/loft --interpret /tmp/other.loft   # recompiles stdlib
make ci   # full suite green → no behavioural drift
```
Expected: `parse_default ≈ 0` for **any** program once the shared
cache is warm; `make ci` green; 100-run wall time drops from 1.72 s
toward the codegen+exec floor.

**Rollback.** `LOFT_NO_CACHE=1` / `--no-stdlib-cache`; cold path
unchanged.

---

### Step 4 — Cache-key completeness (correctness hardening)

**Design.** A wrong key is the only corruption vector (it retired
Phase D once). One `fn stdlib_cache_key() -> [u8;32]` folding **every**
input that changes compiled stdlib output:
- `CARGO_PKG_VERSION` + `LOFT_BUILD_ID`
- `default/*.loft` content
- feature flags that change `Data`/bytecode/registry — `threading`,
  `wasm`, `mmap`, `png`, `native-extensions` (the `#[cfg(feature=…)]`
  entries in `src/native.rs`), as a bitset
- target triple (no cross-arch reuse)

**Implementation.** Single pure function used by all cache paths +
a sensitivity unit test.

**Acceptance test.**
```
cargo test cache_key_sensitivity
#   flip each input → key differs; identical inputs → identical key (deterministic;
#   no Instant/random in the hash)
```

**Rollback.** Pure function; no runtime effect without Steps 1/3.

---

### Step 5 — Default-on + measurement closeout

**Design.** With Steps 1-4 green under `make ci`, flip the
stdlib-prefix cache **on by default** (`LOFT_NO_CACHE=1` escape) and
record the measured win.

**Acceptance test.**
```
# 100-run loop (or hyperfine), default config, vs the 1.72 s/100 baseline
```
Expected: documented before/after (e.g. "1.72 s → X s / 100 runs, N×"),
`make ci` green; this doc + @PLAN28 README updated with real numbers.

**Rollback.** `LOFT_NO_CACHE=1` restores parse-every-run.

---

## Risk register

| Risk | Mitigation |
|---|---|
| Stale cache (the Phase-D bug) | Step 4 key completeness + regression test editing `default/` |
| `Data` non-serialisable fields (`&'static str`, `OnceLock`, pointers) | Step 2a audit; `#[serde(skip)]`+rebuild or owned-String shim |
| CONST_STORE not position-independent | verified position-independent (offset-based `DbRef`); re-confirm in Step 3 with a round-trip-and-run test |
| `native::init` not actually cheap | measured in Step 0; if real, give it its own pointer-free fast path |
| Cache write races (parallel `loft`) | write temp + atomic rename; readers fall back to cold parse on missing/partial |
| Cross-arch/version reuse | target triple + version + build-id in key (Step 4) |

## Sequencing & effort

| Step | Effort | Ships value? | Blocks |
|---|---|---|---|
| 0 Phase attribution | XS | informs design | — |
| 1 Whole-program cache | S | yes (repeated runs) | — |
| 2 `Data` serialization | M | enabler | Step 3 |
| 3 Stdlib-prefix cache | M | **yes (universal)** | 2, 4 |
| 4 Key completeness | S | correctness | 1, 3 |
| 5 Default-on + closeout | XS | yes | 1-4 |

Total ≈ MH, matching the @PLAN28 Phase C estimate. Steps 0-1 are
independently shippable and de-risk the rest; **Step 3 is the
headline** — the universal cold-start win on the always-loaded `default`.

## What "done" looks like

- `LOFT_TIMING` attributes the ~17 ms (Step 0).
- A brand-new, never-seen script starts with `parse_default ≈ 0`
  (Step 3).
- Editing any `default/*.loft` forces a recompile, never stale output
  (Step 4 + regression test).
- `make ci` green throughout; default-on with `LOFT_NO_CACHE=1`
  escape (Step 5).
- This doc + @PLAN28 README carry the final before/after numbers.
