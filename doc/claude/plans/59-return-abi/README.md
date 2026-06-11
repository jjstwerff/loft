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
