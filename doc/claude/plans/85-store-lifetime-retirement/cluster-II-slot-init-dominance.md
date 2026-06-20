<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Cluster II — the #405 live crash (root NOT yet verified)

The LIVE cluster (probe `04`). **⚠️ CORRECTION (instrument falsified the earlier
"verified" root).** While starting Stage D, a backtrace instrument on the bogus
free + a fully-clean IR re-dump overturned the slot-init-dominance story below:

- The failing free is **`OpFreeRef` via `free_ref`** (a plain `OpFreeRef`, the
  per-iteration `OpFreeRef(ki)`), of an **out-of-range store `#24`** while
  `allocations.len==5` — i.e. a garbage `store_nr`, not a double-free of a valid
  store.
- **`__vdb_1` AND `__ref_1` ARE null-init'd at fn entry** in the clean IR
  (`__vdb_1 = null` / `__ref_1 = null` as the first two ops of `main`). So the
  "no scope-entry sentinel-init" claim below is **FALSE**, and a half-1
  fn-entry-sentinel attempt (verified to prepend the init) did **not** fix it.

So the verified facts are only: probe 04 SIGSEGVs on interpret / completes on
native; the crash is an `OpFreeRef` of a garbage `store_nr`; the fn-entry
null-inits are present. The analysis below (slot-init-dominance, half-1/half-2)
is a now-FALSIFIED hypothesis kept only as a record of what was ruled out.

### Pinned so far (VERIFIED by instrument + bytecode, Stage B continuing)

- The crashing op is **ONE `OpFreeRef`, at `code_pos 7292`** (`free_ref` →
  `free` backtrace), firing **per iteration** — i.e. `OpFreeRef(ki)` (the only
  per-iteration free; the other two frees are fn-exit `__ref_1` / `__vdb_1`).
- The freed DbRef is **`{store_nr = 8·(rec−1), rec, pos=1}`** with `rec`
  advancing 1/iter and `store_nr` advancing **8/iter** (24, 32, 40, …) while
  `allocations.len == 5`. The `×8` cadence = a **stack offset / structured
  garbage read as a heap `store_nr`** (the #306 "stack-record ref treated as a
  heap store" shape), NOT a stale-but-valid double-free.
- `ki` (slot 64) is **`PutRef`-assigned** from the `n_enc` return (bytecode
  `Call(n_enc)` → `PutRef(var[64])`), i.e. it ALIASES the NRVO return buffer
  `__ref_1` (enc materialises into the caller-passed `__retbuf`). So `ki` and
  `__ref_1` name the same store, yet BOTH get an `OpFreeRef` (per-iter `ki` +
  fn-exit `__ref_1`).
- Boundary (matrix): fires on **conditional × unused** (`x += ki` whose result
  `x` is never read); disappears when `x` is read (C/E) or assigned
  unconditionally (B).

**Working mechanism (HYPOTHESIZED, needs live confirmation):** when `x` is unused
the `x += ki` copy is dead, so `ki`'s only consumer is dead, and the NRVO-alias
`ki` is freed per-iteration via a malformed DbRef (the `store_nr` half not a real
store) — a double-free / stack-ref-as-heap of the return buffer. This is a
RETURN-BUFFER-aliasing bug (H1/NRVO family), NOT the `__vdb_1` dep-slot story.

**Next step (live inspection — static reads have hit their limit):** `loft debug
--rpc` breakpoint at the per-iteration `OpFreeRef(ki)`, inspect `ki`'s slot bytes
+ what last wrote them; and bisect the boundary (does removing `x += ki` while
keeping `x` unused still fire?) to confirm the dead-copy link.

### CLOSED OUT (debugger `--rpc`, VERIFIED mechanism)

A breakpoint inside the loop showed the frame's live locals as `__ref_1`, `x`,
`__vdb_1`, `i` — **never `ki`**: `ki` renders *as* `__ref_1`, confirming **`ki`
aliases the NRVO return buffer `__ref_1`** (enc materialises its return into the
caller-passed `__retbuf` = `__ref_1`, and `ki = that`). The `BUG #405` fired
*between* the line-9 stops → at the inter-iteration `OpFreeRef(ki)`.

**Verified mechanism:** `ki` is a **borrowed alias of the NRVO return buffer**
`__ref_1`, sharing its store. Under **conditional × unused** (`x = ki` whose
result `x` is never read), scope analysis mis-classifies `ki` as OWNING and emits
a **per-iteration `OpFreeRef(ki)`** — which whole-store-frees the stack-alias,
reading the stack-frame ref's offset as a heap `store_nr` (advancing 8/iter,
`store_nr = 8·(rec−1)`, > `allocations.len`) → #405 / #306 → SIGSEGV. `__ref_1`
ALSO frees the same store at fn exit → it is a **double-free of the return buffer
via a stack-ref**, not a dep-slot-init problem at all.

Evidence chain: matrix (cond×unused boundary) + `free_ref` instrument (OOB
`store_nr` at one `code_pos`, per-iter) + bytecode (`ki` `PutRef`-assigned from
`Call(n_enc)`, the return-buffer alias) + debugger (`ki` renders as `__ref_1`).

### Stage D attempt 2 — witness-guard extension is necessary but NOT sufficient (reverted)

Tried the principled fix: the witness/paired-free block (scan_set ~939) that
handles `ki`↔`__ref_N` aliasing — incl. the **P378(a) `witness_buffer` handler**
for an inner-scoped result var adopting an outer buffer — is gated on
`Type::Reference | Type::Enum`. `ki` is `Type::Vector`, so it was skipped.
Extending the guard to `Type::Vector` **engaged the path** and the SIGSEGV
(double-free) became a **store-table-exhaustion panic** instead.

Why it's only half: the `witness_buffer`/`OpFreeRefIfDistinct` logic makes `ki`'s
per-iteration free **conditional on NOT aliasing** the buffer — and `ki` ALWAYS
adopts here, so it always skips. But `__ref_N` is fn-scoped (reserved once,
freed once at fn exit) while it holds a NEW store every iteration → the
per-iteration stores **leak** → exhaustion. P378(a)'s premise ("the buffer's
fn-exit free covers it") holds only when the buffer is NOT reused in a loop.

**So the complete fix needs the buffer store freed PER-ITERATION at the witness's
inner scope**, not just `ki` skipping. I.e. when `witness_buffer` records `v→av`
with `v` inner-scoped and `av` outer + reused in a loop, `av`'s free (or the
adopted store's free) must be placed at `v`'s scope, freeing each iteration's
store with a VALID ref — combined with the Vector-guard extension. Reverted the
guard-only change (a leak-crash is not a fix). This is a deeper free-PLACEMENT
change in the witness/reclaim path; matrix A–F + leak gate must gate it.

### Stage D attempt 3 — (a)+(b) free-via-buffer is also wrong; the fix is NOT in scope free-placement (reverted)

Tried (a) [Vector guard] + (b) [free the per-iteration store via the BUFFER's
ref in the `witness_buffer` branch, since `ki`'s ref is garbage]. Result:
A/D/F → **#306 "refused to free the stack store (#0)"**, and C/E (previously
clean) → **a new leak**. Decisive finding:

> **Both `ki` AND `__ref_1` are stack-aliases of the real heap store** — the
> NRVO retbuf is itself a stack-record ref. So *neither* var can be whole-store-
> freed: `OpFreeRef(ki)` reads garbage (#405), `OpFreeRef(__ref_1)` hits the
> stack store (#306). The per-iteration heap store's owner is **at the NRVO
> return-ABI level** (the store `enc` allocated, delivered via `__retbuf`), not
> either scope-analysis var.

So the three Stage-D attempts collectively RULE OUT the scope-analysis free-
placement space:
1. fn-entry sentinel (half-1) — insufficient (per-iteration, not fn-exit).
2. Vector witness-guard alone — SIGSEGV → leak/exhaustion (per-iter store unfreed).
3. (a)+(b) free-via-buffer — #306 (buffer is a stack-ref) + breaks C/E.

**The fix is a RETURN-ABI / NRVO-ownership change**, not a `scopes.rs` free
tweak: the store `enc` materialises into `__retbuf` must have a single, valid
owner that frees it per-iteration (the result var must hold the REAL store ref,
or the NRVO delivery must assign ownership cleanly), so that exactly one valid
`OpFreeRef` runs per iteration. That lives in the call/return codegen
(`gen_set_first_ref_call_copy` / the NRVO adoption path, `state/codegen.rs`
~1039-1066 + `parser/control.rs` ref_return), and is a deeper, broad-regression-
risk change requiring the full validation harness. Reverted attempt 3.

### Real-consumer extraction (cbor) — a MUCH simpler/broader cluster-II trigger

Investigating the cbor library's failing `map` test (loft-libs-core branch `cbor`)
distilled cluster II to its minimal, common form — probe
[`05-enum-arg-vector-return-aliasing.loft`](probes/05-enum-arg-vector-return-aliasing.loft):

> **A fn taking an ENUM (payload) arg AND returning a VECTOR, called N times with
> the results held live, returns all-the-LAST-value** — every held result aliases
> the same NRVO return buffer.

```loft
fn enc(c: CV) -> vector<u8> { return match c { CN => [0 as u8], CI { value } => [value as u8] }; }
k0 = CI{value:1}; k1 = CI{value:3}; k2 = CI{value:5};
a = enc(k0); b = enc(k1); c = enc(k2);   // a,b,c == 5,5,5 (interp) / garbage (native)
```

Isolation matrix (all `/tmp`, interp):
- enum arg + **vector** return, ≥2 held results → **FAIL** (all read last). ← trigger
- **integer** arg + vector return → pass (the original `mk(1)/mk(3)` control).
- **vector** arg + vector return → pass.
- enum arg + **text** return → pass.
- recursion (self-referential enum) → NOT required (non-recursive `CV` fails too).
- single held result / one call per loop iteration → pass (only ONE live result).

So this is the SAME root as #405 (NRVO return-buffer aliasing) but a **far simpler
and more common trigger** — no conditional/unused/nested needed, just
`a = f(enumval); b = f(enumval)` where `f` returns a vector. It is why cbor maps
corrupt: `encode(v: CborValue) -> vector<u8>` is called repeatedly in `encode_map`
while holding `ki`. **Both backends** (interp: aliases last; native: garbage `9`).
This both broadens cluster II and gives it a clean, assertion-bearing probe that
the eventual fix must turn green.

### Fix-site localized (VERIFIED) + attempt 4 (parser-materialise, failed)

The IR for `a = enc(k0); b = enc(k1)` shows distinct buffers
(`a = enc(k0, __ref_1)`, `b = enc(k1, __ref_2)`) — yet `a == b` at runtime, so the
aliasing is at the **physical store** level: `a` adopts `__ref_1`'s store but it is
not owned/locked, so `enc`'s next call reuses it. The deep-copy that WOULD give a
call result its own fresh store — `gen_set_first_ref_call_copy`
(`state/codegen.rs`, the `owned_ref` path ~1612-1689) — is gated
`Type::Reference | Type::Enum(_,true,_)` (and the inner branch is `Type::Reference`
ONLY). **A Vector result is excluded, so it never deep-copies.** That is the bug
site: the Reference deep-copy needs a **Vector analog** (allocate a fresh vector
store for `v` + copy the returned vector's elements + do NOT free the source, so
`__ref_N` stays valid for the next call).

**Attempt 4 (reverted):** extend the #410 parser-materialise
(`expressions.rs::native_vec_elm`) to also fire for user-fn vector returns with a
visible Reference/Enum param (`__fwd = enc(k0); a = []; a += __fwd`). FAILED — made
it WORSE (`a=9`, uninitialised garbage): `__fwd` itself is a Vector result that
aliases `__ref_1` (same exclusion), and freeing `__fwd` releases the store the copy
then reads. A parser-level copy through an intermediate doesn't help because the
intermediate has the same bug.

**Conclusion across attempts 1–4** (fn-entry sentinel · Vector witness-guard ·
witness+free-buffer · parser-materialise): the fix is NOT in scope analysis or the
parser — it is the **codegen Vector deep-copy** in `gen_set`'s `owned_ref` path,
mirroring the Reference `OpCopyRecord` path with a vector clone (a fresh owned
store + element copy, source left intact). Load-bearing; gate on probe 05 (both
backends) + the leak/suite harness.

### Attempts 5–6 (parser materialise, refined) — fixes interp, but TWO blockers (reverted)

Extended the #410 `native_vec_elm` materialise (`expressions.rs`) to fire for
**user-fn vector returns with a visible Reference/Enum param**, deep-copying via
`vector_db + OpAppendVector` (the proven `b = a` vector-copy shape). Two variants:
(5) append the call result inline; (6) `vector_db` FIRST, then a skip-free var
temp `__vcp = call`, then `OpAppendVector(v, __vcp)` — order + var-source +
skip-free, all needed.

Result:
- **Interpret: FIXED.** Probe 05 passes (`a=1 b=3 c=5`); the cbor repro is clean.
- **Blocker 1 — REGRESSION:** the `audience_crystal` library tests now hit a
  **`panic at src/compile.rs:306:17`** (+ "parse errors") — the materialise emits
  malformed IR for some shape that library exercises. So the trigger/emission is
  not yet correct for the full shape space (it over-fires or mis-lowers).
- **Blocker 2 — native unaffected:** on native, `a = enc(k0)` returns **garbage
  (`9`) even for a SINGLE call** — the native generator's **vector-return-ABI for a
  ref-param fn is independently broken** (not just aliasing), so a parser-level
  copy of an already-garbage result can't help. Native needs its own
  `src/generation/` fix.

**Net:** the fix is genuinely TWO-backend and harder than a parser materialise:
(i) a parser/codegen vector deep-copy that handles every shape without the
`compile.rs:306` panic, AND (ii) the native generator's vector-return-ABI. Reverted
(a compile-panic regression + an interp/native divergence is worse than the
consistent aliasing). Probe 05 + the audience_crystal compile-panic are the two
gates the real fix must clear.

### Attempt 6 follow-up — `compile.rs:306` was a STALE-CDYLIB false alarm; the real blocker is an elusive audience_crystal regression (reverted)

Re-ran with fresh cdylibs (`make rebuild-native-cdylibs`). Findings:
- **`compile.rs:306` is NOT a compile bug** — it is the "native function not loaded
  (stale cdylib)" runtime stub. The suite hit it because rebuilding loft changed
  `libloft.rlib` and the native cdylibs went stale; rebuilding them clears it. So
  that gate was a false alarm, not something the fix must clear.
- The materialise **does** fix interpret (probe 05 + simple struct/param repros all
  pass) **but regresses audience_crystal**: `update_state`'s
  `parent = assign_cells(snap)` (a `vector<integer>`-returning fn with a struct/
  Reference param) leaves `parent` with "unknown type" at later `parent[i]` uses,
  failing tests 02/03 (which passed on baseline). **It does NOT reproduce
  minimally** — equivalent local-arg, param-arg, and struct-param repros all pass;
  only the real library trips it. That elusiveness is the verdict: the
  parser-stage materialise has subtle two-pass type/scope interactions that break
  real code unpredictably.

**Conclusion:** the parser-stage materialise is the wrong vehicle (works for the
core repro, fragile on real code). The robust fix is the **codegen Vector
deep-copy** in `gen_set`'s `owned_ref` path (post-parse, no two-pass fragility),
plus the **native generator vector-return-ABI** (on native a SINGLE
`a = enc(k0)` already returns garbage `9` — a deeper, independent generator bug).
Both are focused, harness-gated efforts. Gates: probe 05 both backends + the full
suite (audience_crystal 02/03 green, fresh cdylibs).

### Attempt 7 (codegen `gen_set_first_at_tos`) — clean injection point, but the emission is wrong (reverted)

The right injection point IS in codegen, not the `owned_ref` block: `gen_set_first_at_tos`
dispatches first-assignments and has a Reference-from-has_ref_params-call arm
(`gen_set_first_ref_call_copy`) but **no Vector-call arm** — a vector call-result
falls through to a plain adopt. Added a parallel arm + a new
`gen_set_first_vector_call_copy(v, value)` that (1) allocates v's own fresh owned
vector store via `gen_set_first_vector_null`, then (2) generates
`OpAppendVector(v, value, rec_tp)` with `rec_tp = database.content(name_type("main_vector<elm>"))`.

Result: **crashes on interpret** — `index out of bounds: len 71 index 65535` at
`src/database/structures.rs:380`, surfacing inside `enc`. The `65535` is the null
sentinel, so either the `rec_tp` computation is wrong (the `name_type` lookup /
`content()` differs from the parser's `append_elem_tp` = `database.content(vector_of(elm))`)
or the `gen_set_first_vector_null`-then-`OpAppendVector` store/eval-stack
choreography is off (the helper is built for `v = null`, not a call follow-up).
Reverted.

**Remaining detail for the next session:** the injection point + the shape
(`gen_set_first_vector_call_copy`) are right; the bug is in the emission — pin
`rec_tp` against the parser's `append_elem_tp` (use `vector_of`, not a `name_type`
string lookup) and verify the fresh-store + append eval-stack order. Then the
native generator ABI. Gate on probe 05 both backends + audience_crystal 02/03.

## Stage C — fix design: MOVE / ownership-transfer on heap return (not deep-copy)

> **The full, current Stage-C target now lives in
> [stage-c-move-convention-design.md](stage-c-move-convention-design.md)** — the
> move/output-buffer calling convention: invariant, today-vs-correct convention,
> target interpreter bytecode, types, the validated interp prototype, and the
> design→validate→build execution plan. The notes below are the earlier
> formulation that led to it; the design doc supersedes them.

### The real model (why every copy-based attempt fought the grain)

Attempts 1–7 all tried to **deep-copy** the return into the binding. Copy is the
wrong primitive. Other languages get `a = f(); b = f()` right two ways, both of
which share one invariant — **each binding owns a DISTINCT heap object; no
per-call-site buffer is reused while a result is live**:

- **Move semantics** (Rust, C++11, Swift): the return value is *moved* into the
  binding — single owner transfers from callee to caller; the old owner
  relinquishes. Zero copy.
- **GC** (Java, Go, Python, JS): each call returns a reference to a *fresh*
  object; the collector frees by reachability. Aliasing is harmless.
- **NRVO done right** (C++): the elided return is constructed directly into the
  *caller variable's own* storage — and `a`/`b` have distinct storage.

loft is manual single-owner (store + deps + `OpFreeRef` + the per-store
`free_bits` liveness bitmap), so it must follow **move**. Its bug is the
**work-ref reuse optimization**: one `__ref_N` buffer per call site, reused across
calls, sound ONLY when the result is consumed immediately.

### VERIFIED mechanism (allocator instrument, `LOFT_PLN85_OWN`)

For `a = enc(k0); b = enc(k1); c = enc(k2)`, the alloc/free trace is, per call:
`ALLOC #8` (return buffer) + `ALLOC #9` (internal) → **`FREE #9, FREE #8`** →
next call `ALLOC #8` (REUSED). So the return store `#8` is **freed the instant
`enc` returns**, then handed to the next call. `a` keeps pointing at `#8`, which
is now `b`'s. That is the entire bug: **the return store is FREED-on-return
instead of MOVED to the binding.** (Confirms #405 too: same free-on-return, made
visible there by the stack-ref garbage.)

### The fix — apply the ownership model loft already has

The binding must **own** the returned store; the work-ref must **relinquish** it
(no free-on-return). Concretely, ONE of:

- **(C1) Move / ownership transfer.** On `a = <heap-returning call>`, suppress the
  post-return free of the work-ref's store and re-root ownership on `a` (give `a`
  the dep + scope-exit `OpFreeRef`; the store's `free_bit` stays CLEAR so
  `find_free_slot` cannot recycle it for the next call). The work-ref's own
  `OpFreeRef` becomes a no-op (already moved). Zero copy. This is the H3
  "ownership carried, not re-derived" rule applied to the return path.
- **(C2) NRVO into the binding's slot.** Drop the shared work-ref for a return
  bound to a fresh local: have the callee materialize into `a`'s own storage
  (known at the call site for `a = f(...)`). Distinct storage by construction;
  also zero copy. Bigger ABI change but removes the work-ref hazard class
  entirely.

Deep-copy (C3, attempts 5–7) is the conservative fallback the Reference path uses
(safe when the return may alias a caller arg); correct but wasteful, and it hit
the `rec_tp`/store-alloc complexity precisely because copy is the wrong primitive.

### Can we implement it cleanly? — YES, with caveats

The enabling machinery already exists and is clean:
- **Per-store liveness** is an explicit bitmap (`free_bits` + `find_free_slot`,
  `allocation.rs`): keeping the adopted store's bit CLEAR is exactly "don't
  recycle it" — no new infrastructure.
- **Ownership = deps + free responsibility** already exists; the Reference path
  already transfers/owns. C1 applies the same to vector (and any heap) returns.

The work is **localized to the return-bind chokepoint** (suppress the
post-return work-ref free + re-root the dep on the binding), not scattered. The
caveats that make it a *focused* effort rather than a one-liner:
1. **Find the single free site.** The `FREE #8` per call is an `OpFreeRef`
   (`name=''`, via `free_ref`) of the work-ref/return buffer — pin whether it is
   the caller's per-call work-ref free or `enc`'s return cleanup, and suppress
   exactly that one (the existing `paired_witness`/`witness_buffer` + `is_work_ref`
   logic in `scopes.rs` is the place).
2. **Don't regress the immediate-consume case.** When the result IS consumed at
   once (one live result), today's reuse is fine and must stay (no per-call leak).
   The transfer only needs to fire when the result *escapes into a live binding*.
3. **Borrowed-view safety.** When the return aliases a caller arg (the
   `is_borrowed_view` case the Reference path guards), a move would steal the
   arg's store — keep the conservative copy/no-free there.
4. **Native ABI in lockstep.** On native a SINGLE `a = enc(k0)` already returns
   garbage `9` — the generator never transfers ownership either. C1/C2 must be
   mirrored in `src/generation/` or the interp/native divergence persists.

**Recommended path:** C1 (move) for the interpreter first — suppress the
free-on-return + re-root the dep at the `scopes.rs` work-ref chokepoint — gated on
probe 05 + audience_crystal 02/03 + the leak/suite harness; then mirror in the
native generator. C2 is the cleaner end-state but a larger ABI change; do it only
if C1's reuse-vs-escape discrimination proves brittle.

Validate against matrix A–F + the @PLN51/leak gates; confirm the #306 co-fire
closes with it.

### Attempt 8 — C1 via match-arm unification: DIRECTION PROVEN on interp, but 3 gaps (reverted)

Pinned the free-on-return precisely with an allocator instrument (`LOFT_PLN85_OWN`)
+ a `free_ref` code_pos trace: the per-call `FREE #8` is the CALLEE freeing its own
return buffer. The callee's `match`-arm buffers (`__vdb_N`, terminal `_vec_N`) are
NOT unified into one return work-ref, so none is marked `in_ret`, and the taken
arm — the return value — is freed at fn exit (`a=enc(k0); b=enc(k1)` then collide).
A SIMPLE `return [literal]` (`mk`) works because its single buffer IS `in_ret`.

Fix attempted: extend `unify_if_branches_work_refs` (control.rs) +
`returned_var` (scopes.rs) to (a) unify past a value-less `else null` catch-all
(`branch_is_null`), (b) recognise `__vdb_`/`_vec_` arm buffers as unifiable
work-refs, (c) co-unify the `_vec_`↔`__vdb` dep pair, (d) mark absorbed vars
skip-free.

Result — **the move direction is CONFIRMED**: the simple 2-arm **implicit-return**
match (`fn enc(c) -> vec { match c {...} }`, the cbor encode shape) **unified into
the `__retbuf` buffer and works on interp** (`a=1 b=3 c=5`; IR verified: one
`__vdb_1`, wrapped in `return {...}`, no double-free). But three gaps remain:
1. **Native breaks** — the unified IR fails native codegen (E0425, 12 errors): the
   absorbed `__vdb_2`/`_vec_2` is left dangling for the generator (skip-free +
   substitution isn't enough; the var must be fully elided or the generator taught
   the unified shape). C1 must be mirrored in `src/generation/`.
2. **Heterogeneous N-arm** — cbor's real `encode` is a 7-arm match whose arms
   return `head()` CALLS, `{buf; …}` blocks, and nested ifs — not uniform
   `_vec_N`. The pairwise-equal unifier doesn't reduce them; needs a recursive
   collect-all-terminals + unify-to-one (and the call-return arms are themselves
   `__ref_N` work-refs to fold in). So cbor maps still fail.
3. **Explicit `return match`** (probe 05 / x1b) isn't reached — the unify call is
   gated on a bare `If` tail; an explicit `return` makes the tail `Return(If)`.
   Unwrap `Return` at the call site (control.rs:644).

Reverted (native regression). Net: Stage-C's move model is **empirically
validated** (simple match return transfers ownership correctly once unified);
landing it fully = the unifier generalised to heterogeneous N-arm + explicit
return, AND the native generator taught the unified/absorbed shape. The three gaps
above are the concrete checklist.

### Attempt 9 — scoping the full fix re-diagnosed cbor as a DIFFERENT mechanism

Dumping cbor's real `encode` IR overturned the "gap 2 = generalise the unifier"
plan: cbor's `encode` arms **already unify into `__ref_3` (the `__retbuf`)** via
the `one_buffer_chain` / `one_buffer_vec_copy` lowering (head-call arms write the
buffer directly; `buf` arms `OpAppendVector` into it). So the callee side is
correct — attempt-8's match-arm unification (which fixed the SIMPLE literal `enc`)
does not apply to cbor at all.

And the cbor map bug is **not** the held-results aliasing the probes model:
- `encode(CMap{ ONE entry })` is ALREADY corrupt (`[a1, F6, F4]`) — a single
  entry has no two-live-results to alias.
- `k = encode(es[0].key); v = encode(es[0].value)` (two calls, results held) is
  CLEAN (`7, 9`).
- The corruption appears only in `encode_map`'s FULL structure: `buf = head(5,n)`
  + `ki = encode(...)` + the nested `for j` `byte_lt(encode(...), ki)` + the
  `buf += ki` / `buf += encode(value)` appends, all live together.

So the live cbor bug is a **multi-live-heap-value interaction in `encode_map`**
(several `vector<u8>` results — `buf`, `ki`, the `byte_lt` operands, the value
encode — held + appended across nested loops), a store-lifetime mechanism distinct
from BOTH the simple-match callee-free (cluster II, attempt-8) AND the simple
held-results aliasing (probe 05). It has not been reduced to a minimal repro.

**Conclusion of the implementation arc (attempts 1–9):** "the full fix" is not one
change — it is at least three distinct mechanisms (simple-match callee-free-on-
return · the `encode_map` multi-live-value interaction · the native return-ABI that
never transfers ownership), each load-bearing, each needing its own minimal repro +
both-backend validation. Stage C's move model is the right *model* and is validated
for the simplest case, but landing the class is a dedicated multi-step return-ABI
project, not a tail-end change — 9 attempts (this arc) + earlier ones all regressed
or were partial. **Pragmatic ship path for the cbor consumer:** the owned-buffer
workaround in `encode_map` (materialise each `encode()` result into an owned
`vector<u8>` before the next call / append) sidesteps the interaction without the
loft fix; verified on `/tmp/wa.loft` (interp).

---

## (FALSIFIED hypothesis — kept as ruled-out record)

~~Root cause VERIFIED from the IR — this is the shared root.~~ Overturned above.

## Status

| | |
|---|---|
| Root cause | ✅ VERIFIED (IR evidence below) |
| Fix design | candidate chokepoint named (Stage C); implementation = Stage D |
| Severity | corruption + **interpret SIGSEGV** on `main`; native completes (divergence) |

## The mechanism (VERIFIED)

`main`'s inner loop for the #405 repro lowers to (LOFT_LOG=static, `--interpret`):

```
ki = n_enc(i, __ref_1);
x["__vdb_1"] = null;                  // x's null-init (dep = __vdb_1)
if i == t {                           // CONDITIONAL
  OpDatabase(__vdb_1, 65);            //   __vdb_1's slot WRITTEN only here
  x = OpGetField(__vdb_1, 0, 64);
  OpAppendVector(x, ki, 11);
}
OpFreeRef(ki);
...                                   // (fn scope, unconditional:)
OpFreeRef(__ref_1);
OpFreeRef(__vdb_1);                   // ← reads __vdb_1's slot UNCONDITIONALLY
```

`__vdb_1` (the hidden-buffer dep slot) is **allocated conditionally** (inside
`if i==t`) but **freed unconditionally** at fn scope — and its slot is **never
sentinel-initialised at fn entry**. On the `i != t` path the slot holds stale
per-iteration stack content, and `OpFreeRef(__vdb_1)` treats it as a real
`store_nr` → the #405 "refused free of out-of-range store" + the **#306-class**
"stack-record ref treated as an owned heap store" (the slot's stale bytes decode
to a stack-store ref) → **SIGSEGV** on interpret.

| Claim | Status |
|---|---|
| `__vdb_1` allocated only inside the conditional; freed at fn scope | ✅ VERIFIED (IR) |
| `__vdb_1` slot has no scope-entry null-init/sentinel | ✅ VERIFIED (IR — no `__vdb_1 = null` before the `if`) |
| stale slot → bogus `store_nr` → #405 + #306 + SIGSEGV (interp) | ✅ VERIFIED (probe 04 runtime) |
| native completes (init/free imbalance not fatal there) | ✅ VERIFIED (probe 04) — mechanism may still silently corrupt; unconfirmed |
| this is @PLN51 cluster-II's uncovered (conditional × unused × nested) corner | HYPOTHESIZED (matches its shape; not re-bisected) |

## The invariant (the all-paths fix, not the instance)

> **A heap slot's null-init (sentinel) must DOMINATE its free** — i.e. be emitted
> on every path that reaches the `OpFreeRef`, at (or above) the scope where the
> free is placed.

The bug is a **scope mismatch**: `__vdb_1`'s init is placed at the conditional's
local scope (riding the `OpDatabase`) while its free is hoisted to fn scope. The
fix is not "sentinel-init this `__vdb`" (per-instance) — it is to make the
codegen/scope-analysis guarantee the dominance relation for EVERY heap slot:
whenever an `OpFreeRef(v)` is placed at scope S, a sentinel null-init of `v` is
guaranteed at the entry of S (so a skipped conditional allocation frees the
sentinel — a no-op — never stale bytes). That covers every conditional-alloc /
unconditional-free shape at once, retiring the class rather than #405.

This is the runtime-form of the slot-init-before-lifetime-op invariant from
[recent-bugs.md](recent-bugs.md) Finding 3, localised to the free path.

## Localised chokepoint (Stage C — VERIFIED by reading scopes.rs/codegen.rs)

Two facts pin it:

1. **`__vdb_1`'s slot has no null-init of its own.** `x = null` (x's dep =
   `[__vdb_1]`) lowers via `codegen.rs::gen_set_first_*` (the
   `OpInitRef`/`OpInitRefSentinel`/`OpInitCreateStack` block ~1108-1140) to
   `OpInitCreateStack` — which points *x's* slot at `__vdb_1`'s slot but does
   NOT write `__vdb_1`'s slot. `__vdb_1`'s only writer is the **conditional**
   `OpDatabase`; its `OpFreeRef` is unconditional at fn scope.
2. **`scopes.rs` already owns this exact relation** — the Plan-57 cluster-I pass
   (`check`, ~298-318): `store_confinement()` decides a `__vdb` is block-confined,
   then `relocate_null_init()` moves its null-init into that block "so its
   `first_def` / codegen free live there too." The normal (`#410`) IR shows a
   `__vdb_1 = null` BEFORE the `OpDatabase`; the #405 IR has **none** — consistent
   with the null-init being relocated/dropped into the conditional block while the
   free stayed at fn scope.

**One instrument run disambiguates the fix** (do this first in Stage D — don't
theorise): add an `eprintln` behind an env flag in `store_confinement` /
`relocate_null_init` and run probe 04.
- If `store_confinement` returns `__vdb_1` (confined to the `if` block): the bug
  is its **loop/dominance guard** — the `if` block sits *inside* the nested
  loops, which the "non-loop LCA chain" rule (~3979) should already reject; find
  why it doesn't, and tighten so a `__vdb` whose free is NOT inside the candidate
  block is never relocated. (Confinement must imply the free is in the block.)
- If it does NOT fire: the bug is the **codegen gap** — a conditionally-allocated
  `__vdb` with a fn-scope free needs a dominating `OpInitRefSentinel` at fn entry
  (generalise @PLN51's `OpVarRef→OpFreeRef→OpInitRefSentinel` emission to this
  shape).

Either way the enforced invariant is the same (null-init dominates free); the
instrument picks which of the two homes to fix it in. The #306 co-occurrence
should fall out of the same fix (the stale slot decoded to a stack-store ref).

## Stage D — instrument result + the refined fix (in progress)

**Instrument run done** (temporary `eprintln` in `store_confinement`, reverted):
on probe 04 it printed **nothing** → `store_confinement` does NOT classify
`__vdb_1`. So hypothesis (b) confinement/relocation is **RULED OUT**; (a) the
codegen/scope null-init gap is confirmed.

**The chokepoint is `run_scan_phase`'s `lift_vars` prepend** (`scopes.rs` ~144):
it already does exactly the right thing — `for v in lift_vars { bl.operators.insert(0, v_set(v, Null)) }`
— "assigned inside conditional branches but their `OpFreeRef` lives at function
exit; prepend the null-inits so codegen reserves their slot along every path."
But `lift_vars` is populated ONLY by `scan_args` (the `__lift_N` inline-arg path,
~1951). A conditionally-defined `__vdb` freed at fn scope is the same shape and
is NOT added → no prepended null-init → the bug.

**Refinement (the part that makes this load-bearing, not a one-liner):** `x`'s
store is freed **per-iteration** — the pre-Set free of `x = null` reads `x`'s dep
(`__vdb_1`) every loop pass — while `__vdb_1` is REUSED across iterations. So a
single fn-entry `__vdb_1 = null` (the plain `lift_vars` prepend) null-inits only
the FIRST pass; after the store is freed on a later pass, the slot still names the
freed store → a subsequent pre-Set free is a **double-free**. The full invariant
therefore has two halves:
  1. **entry**: the slot is the null sentinel before its first free (the
     `lift_vars` prepend, extended to this `__vdb` shape); AND
  2. **post-free**: a free that consumes a slot **resets it to the null sentinel**,
     so the next (stale) read is a no-op.
Half (2) is the robust, all-paths form (it covers reuse + any future producer);
it is also the higher double-free risk, so it must be matrix-validated, not
guessed. Candidate sites: the free op (`fill.rs`/`Stores::free` — write the
sentinel back after freeing) and/or the `lift_vars` extension for half (1).

**Next concrete step:** boundary matrix varying {conditional, unused, nested,
reuse-count, single-vs-multi-store} on `--interpret` + `--native`; implement
half (1)+(2) at the chokepoint; verify no double-free via the gates below.

## Stage D — boundary matrix + half-1 attempt (DECISIVE: half-2 is required)

**Boundary matrix** (`probes/05-matrix-*`, hand-computed expected = "completes
cleanly", `--interpret`, pre-fix):

| Cell | conditional | unused | nested | result |
|---|---|---|---|---|
| A | ✓ | ✓ | ✓ | **SIGSEGV** |
| B | — (always assign) | — | ✓ | ✅ ok (control) |
| C | ✓ | — (reads x) | ✓ | ✅ ok (control) |
| D | ✓ | ✓ | — (single loop) | BUG #405 (refused, exit 0) |
| E | ✓ | — (reads x) | ✓ | ✅ ok (control) |
| F | ✓ | ✓ | ✓ (8×8) | **SIGSEGV** |

Boundary: **conditional × unused** triggers (D); **nesting escalates to SIGSEGV**
(A/F). B/C/E are passing controls — the matrix is calibrated (can fail AND pass).

**Half-1 attempt (reverted):** extended the `lift_vars` prepend to add a
fn-entry `__vdb_1 = null` for any heap var freed at the top level but defined
only nested. IR confirmed the prepend fired (`__vdb_1 = null` at fn entry) — but
A/D/F **still crashed**. This is the decisive result: the crashing free is NOT
the fn-exit `OpFreeRef(__vdb_1)` — it is the **per-iteration pre-Set free** of
`x = null` (the keyed reassign frees x's prior store via the dep slot every loop
pass). Half-1 fixes the FIRST free only; on a later pass `x = null` frees the
reused store A, leaving `__vdb_1` naming the freed A, so the NEXT `x = null`
**double-frees A** → SIGSEGV.

**Therefore the invariant's half-2 is REQUIRED, not optional:** *a free that
consumes a slot must reset that slot to the null sentinel*, so the next read of
the reused dep slot is a no-op. Fix site = the **keyed-reassign pre-Set free**
(the `x = null` / `OpReplaceKeyed` + `remove_claims` path, `allocation.rs` /
codegen): after freeing x's store, write `DbRef{u16::MAX}` back into the dep slot
(`__vdb_1`). Half-1 (entry sentinel) is still needed for the very first free.

This modifies the reassign/free path for ALL keyed locals → whole-language
double-free risk → it must run the FULL validation harness below, not just the
matrix. Half-1 was reverted (unvalidated + insufficient alone); the design
(half-1 + half-2) is recorded here.

## Stage D (implementation) — validation gates

A codegen/scope change on the free path is load-bearing. Gates: probe 04 +
neighbours (vary conditional / unused / nested independently) green on BOTH
backends; the full matrix; `tests/leak.rs` + the wrap leak gate; a debug-mode
full-suite run; the armed double-free build. Graduate probe 04 (once it no longer
SIGSEGVs) to `tests/scripts/`. Re-run @PLN51's cluster-II probes for no
regression. Verify the #306 co-occurrence is also closed (same root) or split it.
