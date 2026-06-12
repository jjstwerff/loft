<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Plan 59 — the unconditional heap-return ABI (@PLAN59, HOTSPOTS H1)

**Invariant established**: *a function's arity is a pure function of its
declaration.*  Every fn returning `Reference` / `Vector` / struct-`Enum`
carries ONE hidden return-buffer attribute (`__retbuf`, last position,
typed as the return type) from signature parse on — `ref_return` decides
only what FLOWS INTO it, never whether it exists.

## Why (the evidence)

#299 / #306 / @P364 / @P377 / #339 — and the H1 census (2026-06-11)
caught a LIVE #339 sibling on main while probing: vector-literal tails
promote in pass 2 only, and the retrofit had only covered `Reference`
(7-line repro, fixed as plan-59 phase 0).  When a signature depends on
body analysis that depends on other signatures, every new return shape
re-pays the fixpoint tax.

## DESIGN_PROTOCOL — probed claims

- **C1 (probed ✅)**: `ls` (promotable returned locals) is ALWAYS ≤ 1 —
  104/104 promotions across stdlib + scripts + brick-buster + crawler
  (`LOFT_H1_PROBE` census).  Hidden attrs per fn after promotion: 0 or 1,
  never more.  → ONE canonical buffer suffices.
- **C2 (probed ✅)**: late pass-2 growth happens for all THREE heap
  kinds (the census `grew=true` rows were VECTOR fns) — phase 0 closed
  the live hole; the assert in phase 1 makes the class unrepresentable.
- **C3 (read ✅)**: attr↔var binding is BY NAME — `become_argument(v)`
  only flags; the coupling is `attr_names[vars.name(v)]` +
  `function.var(attr.name)`.  → the rebind mechanism is RENAMING the
  promoted local to the canonical attr name.
- **C4 (read ✅)**: a non-promoted callee ignoring its buffer costs the
  caller NOTHING extra at runtime: `add_defaults`' buffer var carries a
  self-dep (`Deps::frame1(vr)`) → `emit_null_dbref`'s `owns_store` gate
  is false → the preamble binds the NULL SENTINEL (no allocation);
  `OpFreeRef` no-ops on it at scope exit.  The by-value copy path
  (`gen_set_first_ref_call_copy`, `0x8000`) keys on the RETURNED deps,
  which stay empty for non-promoted fns → caller behavior unchanged.
- **C5 (consumer census)**: sites keying on hidden-buffer existence —
  `add_defaults` (fills by attr kind ✅ position-blind),
  `collect_hidden_ref_args` (#299, finds by `attr.hidden` + arg shape ✅),
  `nrvo_collapse_tail_set` (first hidden heap attr index ✅),
  `filter_hidden` (✅), `is_borrowed_view` ×2 (returned deps, not arity ✅),
  cdylib export wrappers (`native_lib.rs` marshals the attribute list —
  **C ABI of non-promoted heap returners changes** → cdylib rebuild via
  fingerprint, registry `verified` libs need a re-verify pass),
  fn-ref dispatch + `introspect` (attribute-list driven ✅ follow along).

## Phases

- **Phase 0 (DONE, shipped)** — widen the #339 retrofit to vector/senum
  hidden attrs; regression in `tests/scripts/295`.
- **Phase 1** — signature-time `__retbuf`:
  1. At fn-declaration parse (the point `returned` is known), for
     heap-kind returns: `add_attribute(ctx, "__retbuf", ret)` +
     `hidden = true`.  Idempotent across passes (attr_names re-find).
  2. `ref_return`: the promotion arm RENAMES the promoted local to
     `__retbuf` (vars name-map update) before the existing
     `attr_names` lookup — which now always HITS → `add_attribute`
     becomes unreachable → replace with
     `debug_assert!(false, "arity grew post-signature")`.
  3. Delete `retrofit_callers_hidden_args` + the `grew_in_pass2`
     plumbing (dead once growth is impossible).
  4. `__rref_` recursive-self dance in `add_defaults`: revisit — with
     stable arity it should reduce to the plain path (phase 2 if
     non-trivial).
- **Phase 2** — cleanups: `__rref_`, the name-stability contract
  asserts (H5), doc updates (COMPILER.md fn-ABI section).
- **Phase 3** — validation matrix: full suite both backends + wasm
  rlibs rebuilt; #299/#339/#306/295 regressions; crawler self-test +
  brick-buster `--html` + headless GL gate; `native_library_suite`
  (cdylib ABI); perf spot-check on the benchmark suite (expected ≈0:
  sentinel buffers don't allocate).

## Phase-1 mechanics (designed 2026-06-11, probes C6–C8 pending)

The signature-time attr needs a BACKING VAR (`def_code` builds the callee
frame from vars flagged `argument`, not from the attr list) — so phase 1
creates BOTH at declaration parse: the `__retbuf` attr (hidden, typed as
the return type, last position) and a `__retbuf` argument var right after
the user args (BEFORE any body parsing can intern other vars).

**Probes C6–C8: ANSWERED (2026-06-11, IR dumps + source read)**

- **C6 ✅** — `Function::arguments()` returns argument-flagged vars in
  VAR-NUMBER order; `def_code` frames them in that order.  The implicit
  invariant: *the K-th argument-flagged var (by number) ↔ the K-th
  attribute*.  Today's promoted local (high var nr) therefore lands in
  the LAST slot matching its appended attr; a signature-time `__retbuf`
  var created immediately after the user args aligns by the same rule.
- **C7 ✅** — the caller consumes the result BY VALUE (`x = call(...)`
  binds the returned DbRef); the buffer var is only the allocation
  vehicle.  Cleanup is the witness pair `OpFreeRef(x)` +
  `OpFreeRefIfDistinct(__ref_1, x)` — so a callee REALLOCATING over the
  incoming pointer is already the handled contract (the distinct case
  frees both stores, the identity case frees once).
- **C8 ✅** — the promoted local's `Set(v, Null)` null-init is
  SUPPRESSED once it becomes an argument; the callee's body starts with
  `OpDatabase(s, tp)` ON THE INCOMING SLOT (alloc-from-sentinel / clear
  in place) and `return s` returns the DbRef by value.
  IR: `fn n_full59(a, c, s: P59) -> P59["s"] { OpDatabase(s, 64); … ;
  return s }`.

**Refined promotion mechanism** (replaces the map_nodes rewrite):
because the binding is by NAME (C3) and frame position by NUMBER-order
(C6), promotion becomes a ROLE SWAP — rename the pre-created placeholder
var away + drop its argument flag, rename the promoted local to
`__retbuf` + flag it argument.  The local keeps its var number (all IR
references stay valid); order-of-flagged puts it in the same last slot;
the attr↔var name lookup hits.  No body rewriting needed.

## Phase-1 attempt 1 (2026-06-11): the flip works for plain calls — the
## dispatchers are the remaining work

The three-part implementation (signature-time attr + backing var in
`parse_function`; `ref_return` role swap — ATTR renamed to the promoted
local, placeholder var retired via `Function::retire_argument`; legacy
grow kept for lambda-class defs with a debug assert) — was BUILT and
verified on the plain-call path: the promoted-fn IR is byte-identical
(`fn n_full59(a, c, s: P59)`), a non-promoted fn uniformly becomes
`fn n_reassigned(__retbuf: vector<integer>)`, and the caller-first /
wrapper-chain / closure probes pass on both backends.

The full suite then failed 25 tests, naming the consumers the C5 census
missed — every NON-`add_defaults` dispatcher that derives call arity:

- **C9 — par worker marshalling** (`par_struct_to_*`,
  `par_tuple_return_*`, `par_queue_ref_*`, `wrap::threading`,
  `22c-par-sources` "No elements left on the stack 8 < 12"):
  `generation/ops/parallel.rs` + `src/parallel.rs` size/pass worker args
  from their own tables and never fill the buffer.
- **C10 — cdylib shared dispatch** (`lean_interface_drives_shared_dispatch`,
  `dispatches_struct_return_from_shared_cdylib`,
  `dispatches_data_enum_into_shared_cdylib`): the lean-interface marshal
  builds frames from the attribute list on ONE side only.
- **C11 — completion / capture introspection**
  (`completion_model_resolves_members_from_schema`,
  `capture_heap_types_run_once`): schema-driven signature consumers.

The flip itself is sound; the verified diff is stored as
**`phase1-flip.patch`** in this directory (`git apply` to re-apply).

**THE LOCK, diagnosed (2026-06-11)**: the broken dispatchers classify
hidden attributes BY NAME PREFIX, not by type —
`src/native.rs:1427-1435` (par stitch context):

```rust
n_hidden_text  = attrs.filter(|a| a.name.starts_with("__")).count();
n_hidden_dests = attrs.filter(|a| a.hidden && !a.name.starts_with("__")).count();
```

`__retbuf` is a Reference/Vector-typed dest with a `__` name → counted
as a TEXT buffer (wrong slot kind/size) and missed as a dest → the
`8 < 12` frame underflow.  The same prefix heuristic sits at
`native_lib.rs:60/81/192/383` (the C10 cdylib marshal).  NOTE: this is
a LIVE landmine independent of the flip — a wrapper-promoted attr is
named `__ref_1` (Reference-typed, `__`-prefixed) and would misclassify
in a par worker TODAY.

**The unlock — classify by TYPE, not name** (its own small, gated
change BEFORE re-applying the patch):
- text work buffer ⇔ `RefVar(Text)` (the `is_text_work_buffer` helper
  already exists at `native_lib.rs:81`);
- hidden heap dest ⇔ `a.hidden && (Reference | Vector | Enum(true))`;
- visible user arg ⇔ `!a.hidden` (drop the name tests).
Gates: the 25-test inventory above + a NEW regression for the live
landmine (a par worker calling a wrapper-promoted fn).  C11
(completion/capture) re-check after — likely the same root or a
hidden-filter one-liner.

Order: (1) classify-by-type refactor (standalone PR-able), (2)
`git apply phase1-flip.patch`, (3) the full validation matrix.

**Round 2 (2026-06-11, after the unlock landed)**: the classify-by-type
refactor + par-closure dest emission SHIPPED (the live par landmine is
fixed — `issues::plan59_par_worker_over_wrapper_promoted_callee`); the
flip re-applied reduced the blast 25 → 15 — the cdylib family (C10) is
fully green.  The two remaining lock families:

- **C9 residue — the dest-less par lanes**: `state/mod.rs::execute_at_ref`
  pushes `elem, dests…, extras…` and the ref stitch passes corrected
  counts ✓, but OTHER lanes pass NO dests at all —
  `parallel.rs:981 execute_at` (scalar queue lane) and the
  `execute_at_text` lane.  A heap-returning worker routed through one
  of them post-flip underflows (`8 < 12`): par_queue_ref +
  threading + 22c-par-sources.  Fix: thread `n_hidden_dests` into every
  lane (the A6.a allocation block is reusable), or route all
  dest-carrying workers to the ref lane.
- **C11 — the REPL session machinery** (`repl_interactive_edit_*`,
  `repl_at_frame_*`, `completion_model_*`, `capture_heap_types_*`):
  interactive eval/edit builds call frames with its own arity
  assumptions (`src/repl.rs` / the session store) — needs the same
  hidden-attr awareness or filtering.

Also recorded: plain VECTOR-returning par workers do not compile
natively TODAY (pre-existing queue-shape gap — `closure_shape` buckets
Vector returns as Scalar, `… as i64` on a DbRef; independent of this
plan; belongs to NATIVE.md § N9's par coverage list).

## Risks / open questions

- Variable RENAME helper must update the names map atomically (old name
  lookups for the local: `__ref_1` references in already-built IR are by
  VAR NUMBER ✅; name only matters for attr binding + diagnostics).
- Generic templates (`DefType::Generic`) skip ref_return today (I9-var)
  — they must ALSO skip the signature-time attr, and specialisations
  re-derive it from their concrete return type.
- Coroutines return `iterator<T>` (not heap kinds) → out of scope ✅.
- Text returns use the separate `text_return` machinery — untouched by
  this plan (a future H1b if the same disease shows there).

## PHASE 1 SHIPPED (2026-06-11, round 3)

The flip is IN, with the three final lane fixes the round-2 inventory
predicted plus one it could not see:

1. **threading.rs harness** — `par_queue_ref_adopts_and_rebases`
   hardcoded `n_hidden_dests = 0`; it now derives the count from the
   def like real callers.
2. **par runtime witness-free** (`parallel.rs` ref lane) — a worker
   whose tail built its OWN store leaves the pre-allocated dest
   orphaned; every dest the result did not adopt is freed (mirrors the
   plain-call `OpFreeRefIfDistinct` pair).  Found by the 22c leak gate.
3. **entry-fn dests** (`state/mod.rs::execute_argv`) — the C11 root:
   the REPL's capture wrapper (`fn replmain_N() -> P { … }`) is a
   heap-returning ENTRY fn; the invoker now pushes one null-sentinel
   dest per hidden heap attr so the frame matches.  Fixed the whole
   REPL/completion/capture family.
4. **bridge runtime type-ids** (`native_lib.rs`) — the unforeseen one:
   the shared bridge's HiddenDest fallback embedded the LIBRARY's
   compile-time type id, meaningless in the CALLER's shared store
   (`claim(size=0)` → "Incomplete record" SIGABRT) — and
   `hidden_dest_type_id` was Vector-only anyway.  The bridge now
   resolves the id AT RUNTIME by type NAME (`Stores::name`) in the
   caller's store, covering all three heap kinds.  This path was
   reachable pre-flip only for promoted-callee `;`-decl dispatch and
   simply never had a test.

**Validation (all green)**: full suite 2315/2316 (the env-only
`kernel_port` failure, identical on pure main); crawler kernel
self-test; brick-buster `--html` + the headless GL render gate
(doc/brick-buster.html rebuilt with the flip compiler); n2_cdylib
22/22; debug-mode core suites with the H2 space asserts armed; the
probe battery (caller-first, wrapper chains, closures, REPL captures,
par over promoted callees).

## Phase 2 — DONE (2026-06-11)

- `retrofit_callers_hidden_args` + `grew_in_pass2` DELETED — lambda
  invocations go through CallRef (fn-ref dispatch), never an
  arity-filled `Call`, so the retro-patch could never patch anything
  for the only defs still allowed to grow.
- The `__rref_` recursive-self dance DELETED (`work_refs_recursive` +
  the `work_rref` counter): the signature-time attr exists before any
  body parses, so the promotion re-find no longer depends on work-ref
  numbering.  The `__rref_` name patterns in scopes.rs remain as
  harmless dead halves of string ORs.
- The two-pass contract documented at the `first_pass` flag
  (`parser/mod.rs`); the calling convention documented in
  [COMPILER.md § Function calling convention](../../COMPILER.md).

H1 is RETIRED in STABILITY_HOTSPOTS.  Gates: full suite 2315/2316
(env-only kernel_port), clippy 0, fmt clean.
