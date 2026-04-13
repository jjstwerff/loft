
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Known Problems in Loft

Known bugs, unimplemented features, and limitations in the loft
language and interpreter.  Each entry records the symptom, workaround, and
recommended fix path.

Completed fixes are removed — history lives in git and `CHANGELOG.md`.

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
| 135 | Canvas Y direction not locked in | Medium | **0.8.5:** canonical `(0,0) = screen-top-left`; lock in LOFT.md |
| 137 | `loft --html` Brick Buster runtime `unreachable` panic | High | **0.8.5 blocker:** phase-C bisection of `#native` functions |
| ~~139~~ | `_vector_N` slot-allocator TOS mismatch | — | **Fixed** — `gen_set_first_at_tos` emits `OpReserveFrame(gap)` when the allocator's slot is above TOS (zone-1 byte-sized vars left the gap). Tests: `tests/issues.rs::p139_*` |

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
    JObject { fields: hash<JsonField[name]> },
    JArray  { items:  vector<JsonValue> },
    JString { value:  text },
    JNumber { value:  float not null },   // IEEE-754 per RFC 8259
    JBool   { value:  boolean },
    JNull,
}

pub struct JsonField { name: text, value: JsonValue }

pub fn json_parse(raw: text) -> JsonValue;         // replaces every json_* fn
pub fn to_json(self: JsonValue) -> text;           // round-trip

// JObject / JArray indexers return JNull on miss, never garbage:
pub fn field(self: JsonValue, name: text)  -> JsonValue;   // object only
pub fn item(self: JsonValue, index: integer) -> JsonValue; // array only

// Typed extractors — null on kind mismatch:
pub fn as_text(self:   JsonValue) -> text;
pub fn as_number(self: JsonValue) -> float;
pub fn as_long(self:   JsonValue) -> long;
pub fn as_bool(self:   JsonValue) -> boolean;
pub fn len(self: JsonValue)       -> integer;     // array or object fields

pub fn parse(self: Type, v: JsonValue) -> Type;   // `MyStruct.parse(v)`
```

The old `json_items` / `json_nested` / `json_long` / `json_float` /
`json_bool` surface documented in [PLANNING.md](PLANNING.md) is
withdrawn.  All JSON work routes through `json_parse` → `JsonValue`
from 0.9.0 onward.

Full landing plan in [BITING_PLAN.md § P54](BITING_PLAN.md).

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

## See also
- [PLANNING.md](PLANNING.md) — Priority-ordered enhancement backlog
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — Language design inconsistencies and asymmetries
- [TESTING.md](TESTING.md) — Test framework, reproducing and debugging issues
- [CAVEATS.md](CAVEATS.md) — Verifiable edge cases with reproducers
- [../DEVELOPERS.md](../DEVELOPERS.md) — Debugging strategy and quality requirements
