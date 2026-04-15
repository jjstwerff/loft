
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Known Problems in Loft

Known bugs, unimplemented features, and limitations in the loft
language and interpreter.  Each entry records the symptom, workaround, and
recommended fix path.

Completed fixes are removed — history lives in git and `CHANGELOG.md`.

**Before opening a new issue here, check
[DESIGN_DECISIONS.md](DESIGN_DECISIONS.md)** — the closed-by-decision
register holds items explicitly evaluated and declined (C3 / C38 /
C54.D / …).  If your symptom maps onto one of those, the fix is to
produce new evidence (reproducer, incident, measurement) on the
existing entry, not re-open it as a bug.

## Contents
- [Open Issues — Quick Reference](#open-issues--quick-reference)
- [Unimplemented Features](#unimplemented-features)
- [Interpreter Robustness](#interpreter-robustness)
- [Web Services Design Constraints](#web-services-design-constraints)
- [Graphics / WebGL](#graphics--webgl)

---

## Open Issues — Quick Reference

| # | Issue | Severity | Workaround |
|---|-------|----------|------------|
| ~~22~~ | `spacial<T>` diagnostic wording | — | **Done** — message now says "planned for 1.1+; until then use sorted<T> or index<T>" |
| 54 | `json_items` returns opaque `vector<text>` | Medium | **0.9.0:** first-class `JsonValue` enum (JObject / JArray / JString / JNumber / JBool / JNull); `json_parse` is the one entry point; old text-based surface withdrawn |
| ~~91~~ | Default-from-earlier-parameter | — | **Done** — call-site `Value::Var(arg_index)` substitution in the stored default tree; simpler than planned prologue approach |
| ~~135~~ | Sprite atlas row indexing swap | — | **Fixed** — canonical `(0,0) = screen-top-left`; canvas upload no longer pre-flips rows; OPENGL.md § Canvas coordinate convention.  Regression: 2×2 atlas corner check in `tests/scripts/snap_smoke.sh` / `make test-gl-golden` |
| ~~137~~ | `loft --html` Brick Buster runtime `unreachable` panic | — | **Fixed** — `Instant::now()` guard switched from `feature = "wasm"` to `target_arch = "wasm32"`; `host_time_now()` returns 0 on wasm32-without-wasm-feature; `n_ticks` gated identically. Tests: `tests/html_wasm.rs` (4 regression guards behind a serial mutex) |
| ~~139~~ | `_vector_N` slot-allocator TOS mismatch | — | **Fixed** — `gen_set_first_at_tos` emits `OpReserveFrame(gap)` when the allocator's slot is above TOS (zone-1 byte-sized vars left the gap). Tests: `tests/issues.rs::p139_*` |
| ~~136~~ | wrap-suite SIGSEGV on `79-null-early-exit.loft` | — | **Fixed** — `state/codegen.rs::gen_if` now resets `stack.position` to the pre-if value when the true branch diverges and `f_val == Null`; `is_divergent` recurses into `Insert`/`Block` wrappers (C56 `?? return` puts `Return` inside an `Insert` after scope analysis). Tests: `tests/wrap.rs::sigsegv_repro_79_alone` (un-`#[ignore]`d), `loft_suite` now covers the script. |
| ~~142~~ | `vector<T>` field panics when T is from imported file | — | **Fixed** — plain `use` now imports all pub definitions via `import_all` |
| ~~143~~ | SIGSEGV returning default struct from function iterating nested vectors | — | **Fixed** — `gen_set_first_ref_call_copy` in `src/state/codegen.rs` no longer sets the `0x8000` "free source" bit on `OpCopyRecord`.  The ref-counting that was supposed to make that safe never ran (`inc_rc` defined but unused), so an early-return returning a DbRef through an argument (e.g. `gh_c.ck_hexes[0]` inside the arg `m`) caused the caller to free its own argument's store.  Regression: `tests/lib/p143_{types,entry,main}.loft` + `tests/issues.rs::p143_default_struct_return_from_nested_vector_use`. |

---

## Interpreter Robustness

### ~~86~~. Lambda capture — FULLY RESOLVED (closures shipped)

With real closure capture in 0.8.3, the original codegen error
`[generate_set] ... Var(1) self-reference — storage not yet allocated`
is no longer reachable.  The parser-level mitigation
(*"lambda captures variable X — closure capture is not yet supported"*)
is also gone since the feature is implemented.

The original reproducer now runs correctly end-to-end:

```loft
fn test() {
    count = 0;
    f = fn(x: integer) { count += x; };
    f(10); f(32);
    assert(count == 42);   // passes
}
```

**Regression guards:**
- `tests/issues.rs::p1_1_lambda_void_body` — runtime behaviour (`count == 42`)
- `tests/parse_errors.rs::capture_detected` — parse succeeds, no diagnostic
- `tests/parse_errors.rs::no_capture_no_error` — no false capture positives
- `tests/parse_errors.rs::local_not_captured` — lambda-local vars don't trigger capture

No open action.  Kept here as a marker for CHANGELOG readers; remove on
the next 0.9.0 maintenance sweep.

---

### ~~91~~. Default-from-earlier-parameter — DONE

**Symptom:** `fn make_rect(w: integer, h: integer = w)` fails with
*"Unknown variable 'w'"*; the default expression cannot reference
earlier parameters of the same function.

**Semantics decision:** the default is evaluated *at function entry*,
not at the call site.  That is deliberately different from struct-
field `init(expr)`, which evaluates once at construction.  Required
because the default's whole point is to see the earlier parameters'
call-site values.

**Fix path (three parts):**
1. `parse_arguments` — accept `= expr` referencing earlier params.
   Earlier params are injected into `self.vars` as arguments
   (via `add_variable` + `become_argument` + `defined`) before
   parsing the default, then removed before returning so the
   caller's own argument-registration is unaffected.
2. Call site — pass a supplied-args bitmap (one bit per argument
   with a default) so the callee knows which defaults to evaluate.
3. Function prologue — emit `if !supplied(N) { arg_N = <default> }`
   for each defaulted parameter, using the bitmap bit.

**Scope: M**, three moving parts.  The first naive attempt hit
two-pass state issues in the parser alone; call-site + prologue are
still to do.

---

## Web Services

### 60. Hash iteration — designed 2026-04-13

Full design in CAVEATS.md C60.  Summary: `for e in hash { … }`
iterates in ascending key order, loop variable is the record (no
tuple destructuring).  Implementation is a pre-loop lift that walks
all records of the struct type into a scratch `vector<reference<T>>`,
sorts by extracting key fields, and iterates the sorted vector.
Inefficient by design (O(n log n) per loop); determinism beats
unspecified-order for a scripting language.

Scope: parser routing at `src/parser/fields.rs:599`, a new
`parse_iter_hash` in `src/parser/collections.rs`, a record-walk
helper in `src/database/search.rs` (or reuse the `validate` walk at
line 327), and one new opcode (`OpHashCollect` or `OpHashIterSetup`).

Scope honestly M–MH.  Two days of focused work; the design is
concrete and the scope is bounded.

---

### 54. `json_items` returns opaque `vector<text>` — 0.9.0

**Symptom:** `json_items(body)` returns `vector<text>` where each
element is either a JSON object body or garbage.  The caller writes
`MyStruct.parse(body)` and gets a partial zero-value struct on
malformed input — no type checking, no diagnostic.

**Decision:** replace the text-based JSON surface with a first-class
`JsonValue` enum.  No newtype-around-text half-measure — the newtype
would keep the text surface, its shape predicates would be runtime
peeks into the string, and `.parse` would still run a separate parser
over every element.  Doing the parse once into a typed tree and then
indexing / matching that tree is simpler, faster, and covers the
dynamic-shape use case too.

```loft
pub enum JsonValue {
    JNull,
    JBool   { value: boolean },
    JNumber { value: float not null },   // IEEE-754 per RFC 8259
    JString { value: text },
    JArray  { items:  vector<JsonValue> },
    JObject { fields: vector<JsonField> }
}

pub struct JsonField { name: text, value: JsonValue }

// Parse + diagnostics
pub fn json_parse(raw: text)               -> JsonValue;
pub fn json_errors()                       -> text;     // RFC 6901 path + line:col

// Read surface
pub fn kind(self: JsonValue)               -> text;     // "JNull" .. "JObject"
pub fn len(self: JsonValue)                -> integer;  // null on non-container
pub fn field(self: JsonValue, name: text)  -> JsonValue; // JObject only; JNull on miss / wrong kind
pub fn item(self: JsonValue, index: integer) -> JsonValue; // JArray only; JNull on OOB / wrong kind
pub fn has_field(self: JsonValue, name: text) -> boolean;
pub fn keys(self: JsonValue)               -> vector<text>;
pub fn fields(self: JsonValue)             -> vector<JsonField>; // values deep-copy

// Typed extractors — null on kind mismatch
pub fn as_text(self:   JsonValue) -> text;
pub fn as_number(self: JsonValue) -> float;
pub fn as_long(self:   JsonValue) -> long;
pub fn as_bool(self:   JsonValue) -> boolean;

// Write surface
pub fn to_json(self: JsonValue)            -> text;     // canonical RFC 8259
pub fn to_json_pretty(self: JsonValue)     -> text;     // 2-space indent for non-empty containers

// Construction helpers
pub fn json_null()                                 -> JsonValue;
pub fn json_bool(v: boolean)                       -> JsonValue;
pub fn json_number(v: float)                       -> JsonValue;  // non-finite → JNull
pub fn json_string(v: text)                        -> JsonValue;
pub fn json_array(items: vector<JsonValue>)        -> JsonValue;  // deep-copies items
pub fn json_object(fields: vector<JsonField>)      -> JsonValue;  // deep-copies fields

// Schema-driven (P54 step 5 — pending)
pub fn parse(self: Type, v: JsonValue) -> Type;   // `MyStruct.parse(v)`
```

`JObject.fields` is stored as `vector<JsonField>` rather than the
originally-designed `hash<JsonField[name]>` — the hash form is a
0.9.0 follow-up once hash iteration and nested struct-enum-in-hash
layouts are exercised end-to-end.  Linear scan is fine for the
object sizes typical in configuration / API responses.

The old `json_items` / `json_nested` / `json_long` / `json_float` /
`json_bool` surface documented in [PLANNING.md](PLANNING.md) § H2
is withdrawn.  All JSON work routes through `json_parse` →
`JsonValue` from 0.9.0 onward.

Full landing plan in [QUALITY.md § P54](QUALITY.md#active-sprint--p54-jsonvalue-enum).

---

## Graphics / WebGL

### ~~135~~. Sprite atlas row indexing swap — FIXED

Canvas upload no longer pre-flips rows; `TEX_VERT_2D` samples with
identity V.  Canvas-top = GL TC.y = 0 on all three backends (native
OpenGL, WebGL/wasm, `--html` export), and `lib/graphics/native/src/lib.rs`
+ `lib/graphics/js/loft-gl.js` + `doc/loft-gl-wasm.js` now agree on the
same orientation.  Canonical convention locked in
[OPENGL.md § Canvas coordinate convention](OPENGL.md).

Regression guard: 2×2 atlas corner check added to
`tests/scripts/snap_smoke.sh` — asserts sprite 0/1/2/3 render
red/green/blue/white (matching the atlas's top-row / bottom-row
layout).  `make test-gl-golden` fails if any future upload / shader /
projection change reintroduces a row swap.

Original issue kept below for context.

### 135 (historical). Sprite atlas row indexing swap

**Severity:** Low — cosmetic.

**Symptom:** in a 2×2 sprite atlas, sprites 1 and 3 appear at
swapped canvas positions when drawn via `draw_sprite`.  The smoke
test (`tests/scripts/snap_smoke.sh`) pixel-samples the affected
corners and confirms the mis-placement is reproducible.

**Root cause:** interaction between `gl_upload_canvas`'s Y-flip
(row reversal during upload, `lib.rs:837`), `draw_sprite`'s
V-coordinate computation (`graphics.loft:773-776`), and the
orthographic projection in `create_painter_2d` (`-2/H`, which also
flips Y).  Two of the three flips cancel; the third lands in an
unexpected quadrant, so row indexing into the atlas is off by one
row.

**Workaround:** arrange sprites in a single row (N×1 atlas) until
the flip sequence is normalised.

**Fix path:** decide a single canonical Y direction (screen-origin
top-left) and remove the compensating flip from one of the three
sites — most naturally the upload, since it's the one introduced
last.  Test: extend `snap_smoke.sh` to assert all four corners of
a 2×2 atlas are placed correctly.

---

### ~~137~~. `loft --html` runtime `unreachable` panic — FIXED

Root cause: `Stores::new()` called `std::time::Instant::now()` on the
`--html` build (wasm32-unknown-unknown without the `wasm` feature).
`Instant::now()` panics on this target with no time source; the panic
compiles to `(unreachable)` in release builds, producing the infamous
trap on the very first `loft_start` call — before any user code or
host import ran.

Fix: switch the start-time guard from `#[cfg(feature = "wasm")]` to
`#[cfg(target_arch = "wasm32")]`.  Any wasm32 target uses the
`start_time_ms: i64` field; feature-gated path calls the host bridge,
no-feature path uses 0 as a benign epoch stub.  `n_ticks` on wasm32
without the feature returns 0 (no time bridge, same contract).

Verified: `fn main() { println("hello"); }` compiled with
`loft --html` and instantiated in Node with a `loft_host_print` stub
prints "hello from loft" cleanly.

Test strategy used to find it: debug-built WASM carries Rust panic
string symbols in the stack trace — `noop_debug.wasm` stack showed
`std::time::Instant::now → loft::database::Stores::new` as the panic
origin.  Release builds strip the names and reduce the trap to a bare
`unreachable`, which is why previous diagnostic attempts bottomed out
at "panic in bytecode dispatch, not a host call".

### 137 (historical). `loft --html` Brick Buster: runtime `unreachable` panic

**Severity:** Medium — breaks the deployed `brick-buster.html` on
GitHub Pages; the wasm instantiates but panics as soon as `loft_start`
runs.

**Symptom:** the browser reports

```
Uncaught (in promise) RuntimeError: unreachable executed
    at wasm-function[234]:…
    at wasm-function[229]:…
    …
    at wasm-function[258]:…
```

Reproducible in Node with stub imports: `loft_start()` throws
`unreachable` on the first call, regardless of whether asyncify is
enabled (tested with `wasm-opt -O1 --asyncify` and with no asyncify
pass at all).

**Narrowed down:**

- Not an instantiation failure — all 25 host imports (`loft_gl.*`,
  `loft_io.*`) are present and the wasm compiles.  Pull request #168
  fixed the earlier instantiation-time bug by switching `-Oz` to
  `-O1`; this new failure is at *runtime*, not at instantiate.
- Not a generated-Rust `todo!()` — `grep -c 'todo!'` on the emitted
  `/tmp/loft_html.rs` returns 0.  Every `#native` function has a real
  extern declaration + call.
- Not an asyncify artefact — reproduces with `wasm-opt -O1
  --strip-debug --strip-producers` (no `--asyncify`).
- The panic originates in generated bytecode dispatch, not in a
  host-call — the call stack has no import frames.

**Workaround:** native mode (`make play`) runs the game correctly;
only the browser build is broken.

**Fix path:**

1. Capture the pre-wasm-opt `/tmp/loft_html.wasm` and instantiate it
   directly in Node to confirm the panic is in the rustc output, not
   a wasm-opt transformation.
2. Bisect which `#native` function's return path is unsafe: stub
   each import individually with a `throw new Error(name)` sentinel
   and see which one is hit last before the unreachable — that
   narrows the loft function whose emitted Rust body diverges.
3. Inspect the emitted Rust for that function in
   `src/generation/dispatch.rs::output_native_direct_call` — likely
   a type-marshalling mismatch between the loft signature and the
   generated `extern "C"` prototype (e.g. a `text` param that
   should pass `ptr, len` but was emitted as a single `i32`).
4. Add a browser-path assertion to `make game` that instantiates
   the built wasm in Node and runs `loft_start` against `loft-gl-wasm.js`
   stubs, failing CI if it panics.

**Tracking:** discovered 2026-04-12 while verifying the
`make play` target.  Native path works; browser path wedged.

---

### 138. `--native` rustc E0460: `rand_core` version mismatch

**Severity:** Medium — blocks `loft --native <script>` and `make play`
on a checkout where `cargo build --release --bin loft` has run without
`--lib`.

**Symptom:** `rustc` fails compiling the generated `/tmp/loft_native.rs`
with

```
error[E0460]: found possibly newer version of crate `rand_core` which `loft` depends on
  --> /tmp/loft_native.rs:16:1
   |
16 | extern crate loft;
   | ^^^^^^^^^^^^^^^^^^
   = note: the following crate versions were found:
           crate `rand_core`: …/librand_core-<hashA>.rmeta
           crate `rand_core`: …/librand_core-<hashB>.rmeta
           crate `rand_core`: …/librand_core-<hashC>.rmeta
           crate `loft`: …/libloft.rlib
```

The E0460 cascades: every subsequent `use loft::codegen_runtime::*;`
fails to resolve, producing 700+ "cannot find function `OpNewRecord`"
/ `cr_call_push` / `OpFreeRef` / `n_set_store_lock` etc. E0425 errors.
The generated source itself is fine — rustc can't load the `loft` crate.

**Root cause:** cargo's incremental-build state has `libloft.rlib`
referencing an older `rand_core` rmeta hash than what's currently in
`target/release/deps/`.  This happens when `--bin loft` rebuilds but
`--lib` is left stale.

**Workaround (already shipped):** `make play` step 1 now runs
`cargo build --release -q --lib --bin loft` so the rlib is always
current.  A manual `cargo clean && cargo build --release` is the
fallback when a user's tree has other stale artefacts.

**Mitigation (shipped, `src/main.rs`):** the `--native` driver now
captures rustc's stderr and, on E0460 with "rand_core" or
"possibly newer version of crate", prints an actionable hint —

```
loft: native compilation failed because the cached `libloft.rlib`
references a different dependency version than the one now in
`target/release/deps/`.

Fix:  cargo build --release --lib --bin loft
Or:   cargo clean && cargo build --release
```

This replaces the previous 700-error cascade with a single recovery
instruction.  Test: introduce a stale rlib (`cargo build --bin loft`
after modifying a dependency version) and run
`loft --native <any-file>` — the hint should appear.

---

### ~~139~~. `_vector_N` slot-allocator TOS mismatch — FIXED

**Fix:** `src/state/codegen.rs::gen_set_first_at_tos` now handles
`pos > TOS` by emitting `OpReserveFrame(pos - TOS)` and advancing
codegen's TOS to match.  The runtime stack pointer moves through
the zone-1 byte-sized variable's slot (plain enum or boolean, already
written via `OpPutEnum` / `OpPutBool`), so the subsequent init
opcode writes to the correct zone-2 slot.

**Root cause** (confirmed by trace):
- Slot allocator places byte-sized zone-1 vars (1-byte plain enum,
  1-byte boolean) at fixed slots just below the zone-2 frontier.
- Codegen's TOS counter advances by the op deltas of the per-statement
  push/pop cycle.  `OpConstEnum` pushes 1, `OpPutEnum` pops 1, net
  zero.  The 1-byte zone-1 slot stays "written but not counted in TOS".
- When the next zone-2 `Set(v, …)` runs, slot = zone2_start but TOS =
  zone2_start - 1.  The former `pos == TOS` assert fired.
- Reproducer: plain enum + vector + same-type loop write
  (5 lines — see `tests/issues.rs::p139_enum_vec_same_type_write_through_loop`).

**Why `stack.position = pos` alone failed** (the earlier naive
attempt): the runtime stack pointer wasn't bumped, so subsequent
reads pulled from the zone-1 slot as if it were the zone-2 slot.
`OpReserveFrame` bumps the runtime pointer to match the codegen
pointer.

**Tests:** `tests/issues.rs::p139_enum_vec_same_type_write_through_loop`,
`p139_enum_vec_two_loops_same_function`,
`p139_bool_vec_write_through_loop`.  `tests/wrap::enums` (pre-existing
snapshot test that originally surfaced the bug) stays green.

---

### 139.  *(historical note — see entry above for the fix)*

Discovered 2026-04-12 during C61.local unconditional-reject attempt;
narrowed 2026-04-12 to a 5-line reproducer; fixed 2026-04-13 via
instrumented trace + `OpReserveFrame` in the set-first path.

**Symptom:** codegen panics from `src/state/codegen.rs:922`:

```
[gen_set_first_at_tos] '_vector_3' in 'n_main': slot=N but TOS=N-1
— caller must ensure TOS matches the variable's slot before calling
```

**Minimal reproducer** (plain enum + vector + same-typed
cross-variable assignment inside a for-loop body):

```loft
enum Dir { North, East, South, West }
fn main() {
    dirs = [North, East, South, West];
    first_d = North;
    for elem in dirs { first_d = elem; }
}
```

Trips `slot=N but TOS=N-1` — slot > TOS by exactly 1 byte, matching
the enum discriminant size.  An alignment gap the allocator reserved
(for the vector temp `_vector_N`) that the TOS counter didn't advance
through.

**Not a simple "advance TOS" fix:** naïvely setting
`stack.position = pos` in `gen_set_first_at_tos` (the mirror of the
existing `pos < TOS` correction) makes the assert pass but produces
garbage at runtime (`index out of bounds: the len is 4 but the
index is 768`).  The padded byte isn't actually free — it's either
initialised by a prior op the allocator expected to run or the
slot was pre-assigned without accounting for the enum's 1-byte
discriminant.

**Real fix path:** phase-B dump at the `_vector_N` creation site —
what op produces the slot offset?  what writes into the alignment
gap?  The assert is only the symptom; the root is in either
`src/variables/slots.rs` (slot pre-assignment not accounting for
byte-sized discriminants) or one of the `OpNewVector*` emit sites.

**Why it matters now:** blocks C61.local's stdlib rename sweep.
Latent in main today — no CI exercises the triggering layout — but
independently reproducible via the enum + for-loop snippet above.

**Discovered:** 2026-04-12 during C61.local unconditional-reject
attempt (commit b716d1d, reverted).  Narrowed 2026-04-12 via a
5-line reproducer and a failed naïve fix.

---

## 136. Wrap-suite SIGSEGV on `79-null-early-exit.loft` — FIXED

**Root cause.** `state/codegen.rs::gen_if` (the `f_val == Value::Null`
branch) left `stack.position` at the true-branch's end-state after emitting
a divergent true branch.  At runtime the join point is reached only via
the `OpGotoFalseWord` jump, where `stack_pos` equals the pre-if value —
so every subsequent `Var*` / `Put*` op encoded `var_pos = codegen_stack −
slot` was 4 bytes off.  Writes through `_ncr_1` / `val` corrupted the
return-address slot; after a handful of `safe_double` calls the
interpreter read a small bytecode offset as a return address and
re-entered already-returned code, growing the stack by ~12 bytes per
iteration until it overflowed the 8008-byte stack store.

`is_divergent` also did not recognise `Value::Insert([..., Return(...)])`
— the shape `scopes.rs` produces when it wraps a `Return` with
`free_vars` cleanup.  So even the else-present branch's divergence reset
(line 520-524) silently missed the C56 case.

**Fix.** Two small edits in `src/state/codegen.rs`:
- Widen `is_divergent` to recurse into the last op of `Value::Insert` and
  `Value::Block`.
- In the `*f_val == Value::Null` arm of `gen_if`, reset
  `stack.position = stack_pos` when the true branch is divergent.

**Tests.**  `tests/wrap.rs::sigsegv_repro_79_alone` is no longer
`#[ignore]`d; `tests/wrap.rs::loft_suite` now runs
`79-null-early-exit.loft` (previously skipped via `ignored_scripts()`).
Passes debug + release, and under `target/release/loft --interpret`.

---

## 136. (historical) Wrap-suite SIGSEGV on `79-null-early-exit.loft`

**Severity:** High (release blocker — see RELEASE.md Gate Items).

**Symptom:** `cargo test --release --test wrap` (or the full suite
`./scripts/find_problems.sh`) aborts with one of:
- `free(): invalid pointer`
- `corrupted size vs. prev_size`
- `signal 11 SIGSEGV: invalid memory reference`

Always attributed to `loft_suite`, which runs every
`tests/scripts/*.loft` sequentially through `wrap::run_test`.
The wrap `loft_suite` now **skips `79-null-early-exit.loft`** via
`ignored_scripts()`, but the script is STILL covered by a
dedicated `#[ignore] sigsegv_repro_79_alone` regression test —
that test currently crashes when run (`--ignored`), locking the
reproducer for the eventual fix.

**Not** caused by this session's P54-U changes.  Still reproduces
after `git show HEAD:src/*` replaces every modified `src/` file
with its committed HEAD content.  The bug is pre-existing at
commit `d0d6932`.

**Debugger fingerprints (valgrind + crash reporter):**

```
Invalid write of size 1
   at loft::fill::op_return
   by loft::state::State::execute_argv
 Address ... is 8 bytes after a block of size 8,008 alloc'd
   by loft::state::State::new
```

In a debug build the bounds check fires earlier:

```
thread 'sigsegv_repro_79_alone' panicked at src/store.rs:902:9:
Store read out of bounds: rec=1 fld=8005 size=4 store_size=8008

=== loft crash (wrap) SIGABRT caught ===
  last op:  (opcode dispatch) (op=5)
  pc:       0
  fn:       (?) (d_nr=4294967295)
===
```

The 8008-byte block is the stack store allocated in `State::new`
(`db.database(1000)` → 1000 words × 8 bytes).  `op_return` (op=5)
writes 8 bytes past the end of that block — `stack_pos` climbs
above 8000.  Live instrumentation shows `fn_return` being called
repeatedly at `code_pos=6` (or 12 / 18), reading `u32::MAX` but
getting `6` / `12` / similar small bytecode offsets, turning the
wrap-test binary into an infinite loop that grows the stack by
12 bytes per iteration until it overflows into adjacent heap and
corrupts Rust's allocator metadata.  The `Data::drop` at end of
`run_test` then finds corrupted `Value`/`String` entries and
glibc aborts.  `call_stack` is empty by the time the loop runs
(d_nr=u32::MAX in the crash report) — execution has already
left main and is "returning past the bottom of the stack".

**Runs fine via CLI:**

```
$ target/release/loft tests/scripts/79-null-early-exit.loft
  (exits 0, clean)
$ valgrind target/release/loft tests/scripts/79-null-early-exit.loft
  (zero memory errors)
```

So the bug lives somewhere in the difference between
`cached_default()` → clone → `run_test` vs. a fresh
`parser.parse_dir` → parse user file → execute.

**Leading hypotheses (unverified):**

1. **Frame-yield residue from a default-parse side effect.**  The
   default library's parser pass registers some lazily-initialised
   state (static `NATIVE_REGISTRY`, closure maps, etc.).  If the
   cached clone differs subtly from a fresh parse — a differently-
   sized stack reserve, a const-store offset, an unset `arguments`
   register — main's `OpReturn` could read its ret/discard operands
   off the wrong bytecode position and corrupt the stack.
2. **C56 `?? return` interaction with top-level return.**  Script
   79 is the ONLY script in the suite using `?? return`.  The
   desugared form emits an inner `OpReturn` inside `safe_double`
   / `chain_test` / `void_test`.  A compile-time mismatch between
   `self.arguments` (cached at def_code entry) and the current
   stack.position at the nested `Return` could land us at wrong
   offsets on return.
3. **Stale `self.arguments` between functions.**  `self.arguments`
   is a `State` field mutated inside `def_code`.  If a previous
   def's value leaks into another def's `gen_return`, the bytecode
   for that return has the wrong `ret` operand.

**To reproduce:**

```
cargo test --release --test wrap sigsegv_repro_79_alone -- --ignored --nocapture
```

**Debug aids already in place** (no setup needed for next session):

- `src/crash_report.rs` — `install("loft")` is called from
  `src/main.rs` startup; `install("wrap")` is called from
  `tests/wrap.rs::run_test`.  The interpreter's execute loop in
  `src/state/mod.rs::execute_argv` calls `set_context(pc, op_code,
  op_name, fn_d_nr, fn_name)` at every opcode dispatch.  On
  SIGSEGV/SIGABRT/SIGBUS the handler async-signal-safely prints
  the published context to stderr, then the default handler runs
  to produce the core dump.
- `tests/wrap.rs::sigsegv_repro_79_alone` (`#[ignore]`) is the
  standalone reproducer; `tests/wrap.rs::ignored_scripts()`
  skips `79-null-early-exit.loft` from `loft_suite`.
- `ulimit -c unlimited` + `sysctl -w kernel.core_pattern=/tmp/core.%e.%p`
  — local core dumps, inspect with `gdb -c core target/release/deps/wrap-<hash>`.
- `valgrind --error-exitcode=42 --track-origins=yes --num-callers=30
  target/release/deps/wrap-<hash> sigsegv_repro_79_alone --ignored
  --nocapture` — points `op_return` at the out-of-bounds write.

**Discovered:** 2026-04-14 during P54-U phase 2 test sweep.
Reproduces at `d7ef549` (`origin/main` after PR #170 merge); was
also reproducible at the pre-merge `d0d6932` commit.
See `CHANGELOG.md` and `doc/claude/RELEASE.md` § "Crashes" for
release-block ownership.

---

## Package / Multi-file

### 142. `vector<T>` field panics when T is a struct from an imported file

**Severity:** High — blocks multi-file library layout for any package that
uses `vector<StructType>` fields where the struct is defined in a separate
`.loft` file.

**Symptom:** The parser panics with:

```
assertion `left != right` failed: Unknown vector unknown(N) content type on [M]Outer.field
  left: 4294967295
 right: 4294967295
```

at `src/typedef.rs:311` during the type-fill phase (`fill_all`).

**Reproducer (minimal):**

```
# inner.loft
pub struct Inner { val: integer not null }

# outer.loft
use inner
pub struct Outer { items: vector<Inner> }
fn test_it() {
  o = Outer { items: [] };
  assert(len(o.items) == 0, "empty");
}
```

Run: `loft --lib <dir-containing-inner> outer.loft` → panic.

The identical code in a single file works without issue:

```
struct Inner { val: integer not null }
struct Outer { items: vector<Inner> }
```

**Root cause (likely):** `typedef.rs::fill_all` resolves `vector<T>` content
types during the type registration loop.  When `T` is a struct loaded via
`use` from a different file, the struct def-nr is not yet known at the point
where the vector content type is resolved — the two-pass design fills types
file-by-file, so cross-file struct references in vector generics see
`u16::MAX` (4294967295) instead of the real def-nr.

**Workaround:** Put all structs that reference each other via `vector<T>`,
`hash<T>`, `index<T>`, or `sorted<T>` in the same `.loft` file.  This is
sufficient for the Moros `moros_map` package (all types in one file).

**Discovered:** 2026-04-14 while implementing MO.1a (Moros hex scene map
data model).  The designed layout had `types.loft`, `palette.loft`, and
`spawn.loft` as separate files with `Map` referencing all of them via
`vector<T>` fields.

---

### ~~143~~. SIGSEGV returning default struct from function iterating nested vectors — FIXED

**Status:** Fixed 2026-04-15 — `gen_set_first_ref_call_copy` no
longer tells `OpCopyRecord` to free its source.

**Root cause:** `src/state/codegen.rs::gen_set_first_ref_call_copy`
unconditionally set the `0x8000` bit on the type parameter passed
to `OpCopyRecord`, which the interpreter reads as "also free the
source store after deep-copying".  The original comment claimed
that was "safe with store reference counting", but `inc_rc` in
`src/database/allocation.rs:146` is defined and never called — every
store lives with `ref_count = 1` and the rc-gated branch of `free`
is unreachable.

When a ref-returning function takes an early-return path that
returns a DbRef *aliasing one of its own arguments* (e.g. inside a
`for`-loop: `return gh_c.ck_hexes[0];` — a DbRef into the caller's
`m` via the iterator element), the 0x8000 path freed the caller's
argument.  The next call on the same argument dereferenced a freed
Store and SIGSEGV'd — observed most reliably in integration tests
where the allocator handed the freed slot back to unrelated data
before the next read (CLI binary happened to get valid-looking
garbage).

**Fix:** Drop the `| 0x8000` in `gen_set_first_ref_call_copy`.  The
caller-allocated work-ref that backs the callee's hidden `__ref_1`
buffer is no longer freed by the copy op itself — instead it stays
alive until `scopes::free_vars` sweeps the caller's frame at
function exit.  A narrow bounded leak strictly preferable to a
use-after-free.

**Regression:** `tests/lib/p143_types.loft`,
`tests/lib/p143_entry.loft`, `tests/lib/p143_main.loft` exercise
three distinct IR shapes — empty-map fallback, found-on-first-
chunk, and loop-fallback-after-miss.
`tests/issues.rs::p143_default_struct_return_from_nested_vector_use`
asserts the interpreter sees no `had_fatal`; the native path is
exercised by the same fixtures via the script runner.

**Discovered:** 2026-04-14 while implementing MO.2 (moros_map
serialization).  Latent since at least 2026-04-13 (P142/P137 cluster).

---

## See also
- [PLANNING.md](PLANNING.md) — Priority-ordered enhancement backlog
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — Language design inconsistencies and asymmetries
- [TESTING.md](TESTING.md) — Test framework, reproducing and debugging issues
- [CAVEATS.md](CAVEATS.md) — Verifiable edge cases with reproducers
- [../DEVELOPERS.md](../DEVELOPERS.md) — Debugging strategy and quality requirements
