<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN85 / D-own-1 — collapse the `block_result` return-delivery thicket

The ownership code-simplification exploration ([OWNERSHIP_MODEL.md § ACTIVE](../../OWNERSHIP_MODEL.md#active--the-simplification-exploration-next-days-exploratory--revertable)),
first slice. Exploratory + REVERTABLE: land a collapse, validate it identical
across both backends with the @PLN89 oracle, keep it if it shrinks the thicket
without regressing — revert if it doesn't pay off.

## The instrument (re-assertion-site count — design-protocol step 2)

`Parser::block_result` (`src/parser/control.rs`, ~lines 573–1032) is **459 lines,
45 special-case helper calls, 15 distinct tail-shape decision helpers**:

```
14 ref_return            3 text_return                  1 tail_whole_arg_vector
 5 nrvo_collapse_tail_set 3 materialize_vector_arms_into 1 tail_terminal_fresh_local_vec (#448)
 4 return_buffer          2 tail_if_has_null_arm          1 tail_is_struct_field_read
                          2 collect_hidden_ref_args       1 returned_uses_buffer (#448)
                                                          1 body_has_buffer_return (#448)
                                                          1 materialize_view_return
                                                          1 callee_forwards_foreign_store
```

Each helper answers ONE question — *which store does this return deliver, and who
frees `__retbuf` vs the returned store* — by re-inspecting the parse-tree SHAPE of
the tail (is it a branch? a bare `return Var`? an arg borrow? a struct-field
read? a call that forwards a foreign store?). The same question, re-derived ~15
ways. #448 added three more helpers to fix one leak — the tell that the structure,
not the logic, is the burden ([[evolve-data-structures-when-burdened]]).

## The deviation (formal/ownership.md D-own-1)

> Ownership is re-derived per-site by codegen, not carried as one `deps` fact.
> Each fix added a codegen condition rather than completing a fact.

The typed `Deps` substrate (D-own-3) is DONE — the fact is now typed and
readable. So the cure is finally available: have these sites READ the deps fact
instead of re-deriving the tail shape.

## The ONE invariant (design-protocol step 4)

> **The return-delivery decision — which store a `return` yields, and whether the
> caller frees `__retbuf` or the returned store — derives from ONE deps fact
> computed once per return binding, not re-derived per tail-shape at the delivery
> site.**

When this holds, the 15 shape-classifiers collapse to one read: a return either
(a) already writes `__retbuf` (deliver as-is), (b) owns a fresh store (move it
into `__retbuf` / rename), or (c) borrows a visible source (copy into `__retbuf`)
— and *which* is a property the deps fact already encodes (owned vs the borrow
source), not a shape to re-classify.

## First collapse target

The **vector return-buffer sub-thicket**: `return_buffer` + the 3
`materialize_vector_arms_into` sites + the #448 trio (`returned_uses_buffer`,
`body_has_buffer_return`, `tail_terminal_fresh_local_vec`) + `tail_terminal_is_branch`.

### Refined finding (from reading the whole `t == Vector(elm, ls)` arm, ~785–960)

The decision is **already deps-branched at the top** — the code keys off `ls` (the
tail Type's deps): `ls.is_empty()` → OWNED-FRESH; `ls.iter().any(is_argument)` ∧
(struct-field ∨ whole-arg) → BORROWS-ARG; else → multi-arm. So the gap is NOT "the
fact isn't read" — it IS read to branch. The gap is that the **three delivery
MECHANISMS are unfused**, each re-handling shape inside its branch:

| mechanism | what it does | when |
|---|---|---|
| **rename** (`ref_return` + `nrvo_collapse_tail_set`) | promote the tail's local to BE `__retbuf` | owned-fresh, buffer FREE |
| **copy** (`OpClearVector`+`OpAppendVector`+free) | clear `__retbuf`, element-append, free the source | borrows an arg, OR owned-fresh but buffer TAKEN (#448) |
| **forward-copy** (`native_forwarder`) | mint a local, run the foreign-store call into it, copy in | a `#native` callee that delivers its own store |

`#448` is exactly "owned-fresh but the buffer is TAKEN by an early Call return →
must **copy**, not rename" — a fourth path bolted on because rename was assumed
whenever owned-fresh. **The collapse is the SELECTOR, not the reads:** one
`return_delivery(ls, buffer_taken, terminal_kind) -> {Rename, Copy, ForwardCopy, AsIs}`
computed once from the deps fact + buffer-availability, then ONE dispatch to the
three mechanism emitters — so #448's "buffer taken ⇒ copy" becomes a cell in the
selector, not a special case, and the per-branch shape re-handling collapses.

## Probe before building (design-protocol — falsify the hypothesis cheaply)

Before touching the code, confirm on the oracle corpus + the matrix that the deps
fact is ALREADY sufficient to distinguish the cases the shape-classifiers split on
(owned-fresh vs already-`__retbuf` vs arg-borrow). If a case is distinguishable
only by shape and NOT by any deps fact, that gap is a D-own-2 (incompleteness) to
close first — the collapse cannot be wider than the fact is complete.

## Safety net + landing rule

Every step: full suite both backends + the @PLN89 oracle `--ignored` sweep
(leak/value/halt identical) + the `tests/leak.rs` + wrap leak gate. Collapse one
sub-thicket at a time; if a step regresses, bisect by site and revert that site
(the #448 first cut regressed `104-split-text` — the matrix caught it). The win is
measured in deleted helpers + shrunk line count, with zero behaviour change.

## Probe result (deps-sufficiency CONFIRMED — from the #448 bytecode)

The `LOFT_LOG=fn:n_pick` dump of the #448 repro already answers the probe:

- the leaking tail is `_vec_1(4):vector<integer>["__vdb_1"]` — **deps `["__vdb_1"]`**:
  it OWNS a fresh local store, distinct from `__retbuf`;
- the early Call arm is `__retbuf(0):vector<integer>` — **delivers `__retbuf`**;
- `returned` carries the `__retbuf` attr.

So the deps fact *already* distinguishes the three cases the shape-helpers
re-derive: **owned-fresh** (deps = a `__vdb` local ≠ buffer), **delivers-buffer**
(deps = `[__retbuf]`), **arg-borrow** (deps = an arg var). Two of the three #448
helpers (`returned_uses_buffer`, `tail_terminal_fresh_local_vec`) are ALREADY
reading deps — they just re-walk the shape to find the terminal first. The
collapse is therefore sound and not blocked by a D-own-2 gap: **one descent to the
return terminal + one deps read** replaces the per-shape classification. (Empirical
re-confirm on the full oracle corpus is the first step of the build, but the
representative case is settled.)

## Status

- [x] Instrument: the thicket counted (459 lines / 45 calls / 15 helpers).
- [x] Invariant named; first target + probe scoped.
- [x] Probe the deps-sufficiency — CONFIRMED on the #448 case (owned-fresh /
      delivers-buffer / arg-borrow are all deps-distinguishable).
- [x] Read the whole `t == Vector` arm — REFINED the target: the decision is
      already deps-branched; the gap is the THREE unfused mechanisms
      (rename / copy / forward-copy), with #448 a fourth bolt-on. The collapse is
      the SELECTOR, not the reads.
- [x] **Build slice 1 — the `Delivery` selector for the lower `t == Vector` arm**
      (commit `cc69101b`). `enum Delivery { Rename, CopyBorrow, ForwardCopy, AsIs }`
      + `classify_vector_delivery` (pure `&self`) + `dispatch_vector_delivery`; the
      #409 forward-copy block extracted to `emit_forward_copy_409`. The three inline
      branches (recover-hidden-refs / arg-borrow-copy / multi-arm-rename) are now
      cells of one selector. **Behaviour-PRESERVING**: `loft introspect` over the
      `bytecode-comparisons/D-own-1-corpus.loft` (one fn per delivery path) is
      byte-identical IR + native Rust before/after, both backends. Net +44 lines
      (scaffolding); the measurable shrink lands when #448 folds in (slice 3).
- [x] **The #448 c5 residual** (commit `237b8347`) — surfaced mid-slice by a user
      boundary matrix. An explicit `return [literal]` left `returned` a BARE vector
      (vs the implicit tail's `["__retbuf"]`); a bare-returning vector fn CHAINED by
      an NRVO caller (`return wrap()` → `__retbuf = wrap(__retbuf)`) orphans the
      store `wrap` never wrote there. Broader than the filed matrix: multi-return
      bare literals (`wrap(dual)`) leak the same way, both paths. Fix delivers EVERY
      fresh-owned vector return into `__retbuf` — tail (#437 intercept generalized
      to literals via `fresh_owned_vector_deps`, gated `!vec_arm_handled` against the
      #448-path double-append) AND mid-body (`deliver_mid_vector_returns`). This is
      the `parse_return` residual the plan anticipated. Matrix (value+len+leak, 13
      cells) clean both backends; corpus stays byte-identical (surgical); suite +
      oracle green; guarded by `tests/oracle/09-nrvo-bare-return-chained.loft`.
- [x] **Slice 3 — fold #448 into the tail-return cell** (commit `c9b8f154`,
      byte-identical, net −29 lines). The #448 buffer-taken delivery was a second
      upper materialise block with its own three-helper gate; it is now ONE cell of
      the fresh-owned-vector tail-return handling (the #437/c5 block): the deps fact
      + buffer state decide rename-vs-copy — `fresh_owned_vector_deps(tail)=Some` →
      buffer FREE renames, buffer TAKEN materialises (#448). `tail_terminal_fresh_-
      local_vec` DELETED (subsumed by `fresh_owned_vector_deps`); `returned_uses_-
      buffer` + `body_has_buffer_return` stay as the buffer-taken cell-guard
      (legitimate, not bolt-on). Moving #448 past `convert` is sound (a `Never`-typed
      `return <expr>` tail is inert to convert). Corpus byte-identical both backends;
      13-cell value+len+leak matrix all pass; suite + oracle green.
- [x] **#448 mirror — mid-body delivery via a tail call-chain** (commit `0f79737b`).
      Surfaced by sweeping the class across composition axes. An early
      `return [literal]` (mid-body) + a tail `return <call>` NRVO-chain: the chain is
      a `parse_return` `MidReturn`, which (unlike a tail rename) never triggers
      `deliver_mid_vector_returns`, so the deferred early literal orphans its store on
      that path. PRE-EXISTING on `main` (classified vs an `origin/main` worktree), the
      mirror of c5. Fix: a final `block_result` cell delivers every mid-body return
      when the fn is buffer-bound but no tail cell handled it (the call-chain case);
      cells that DID handle the tail short-circuit it, so no double-delivery. Corpus
      byte-identical; suite + oracle green; guard = oracle prog 09 (extended).
- [x] **Class swept dry.** ~41 probes / 5 rounds: round 1 caught the mirror, rounds
      2–5 dry. Axes: literal · comprehension · call · arg-borrow · struct-field ·
      empty · vector-of-structs · nested vectors · deep/nested chains · three-way
      mixed · loop reuse · recursion · methods (`self:`) · generics · fn-refs ·
      explicit+implicit-tail mix · match arms — and the PARALLEL `__retbuf` arms
      (struct/Reference, text, struct-enum), all clean both backends (the gap was
      vector-specific). The vector-return-delivery leak class is closed.
- [x] **Dispatch unification — #416 + #448 through the ONE vector dispatch**
      (commit `0fcf66fa`, byte-identical). The branch-tail materialise (#416,
      `vec_match_candidate`) and the buffer-taken materialise (#448 cell) both called
      `materialize_vector_arms_into` + set `returned` inline; both now route through
      `dispatch_vector_delivery` as `Delivery::Materialize`. The dispatch returns
      whether it delivered (the #416 caller gates `vec_arm_handled`, the #448 caller
      its fallback rename). Every vector-delivery mechanism now flows through ONE
      dispatch (Rename / CopyBorrow / ForwardCopy / Materialize / AsIs). Corpus
      byte-identical both backends; matrix + hunt + suite + oracle green.

## Close-out — the collapse paid off

The vector return-buffer sub-thicket is collapsed to the plan's invariant:
**the deps fact decides (`fresh_owned_vector_deps` / `ls`), ONE dispatch emits.**

- **Structure.** The lower implicit-tail arm is `classify_vector_delivery` (pure
  `&self`) → `dispatch_vector_delivery`; the explicit-return tail (#437/c5) and the
  upper branch tail (#416) + buffer-taken (#448) all produce a `Delivery` routed to
  that one dispatch. Classification stays at three structurally-distinct entry
  points (implicit tail / explicit `return` / pre-convert branch) — that split is
  inherent (different `t`, different convert-ordering), not re-derivation — but the
  *mechanism* is no longer re-handled per branch.
- **Shrink.** `tail_terminal_fresh_local_vec` deleted (subsumed by
  `fresh_owned_vector_deps`); the #448 second upper materialise block removed; the
  per-branch inline mechanism handling replaced by one enum + one dispatch.
- **Bugs found + fixed (the real payoff).** Two pre-existing `main` leaks in the
  `#448` class the collapse surfaced and closed: **c5** (explicit `return [literal]`
  never delivered → orphaned when chained) and **h4** (mid-body literal not
  delivered when buffer-bound via a tail call-chain). Both guarded by oracle prog
  `09`.
- **Swept dry.** ~41 probes / 5 rounds across all axes incl. the parallel
  struct/text/enum `__retbuf` arms — the vector-return-delivery leak class is closed.

The Reference / Text return sub-thickets are a *future* D-own-1 slice (their arms
are clean today — round 5 — so any collapse there is organisational, not
bug-driven). This slice (the vector sub-thicket) is DONE.

## Slice 2 — the Reference (struct) return sub-thicket (2026-06-26)

The close-out above named the Reference/Text arms a *future* slice. Reference done now,
mirroring the vector collapse: `RefDelivery { Rename(ws), MaterializeView, AsIs }` +
`classify_reference_delivery` (pure `&self`) → `dispatch_reference_delivery`. The three
inline sub-cases of the `Type::Reference(td, ls)` arm (#120 hidden-ref recovery / #306
return_views_local copy / plain rename) are now cells of one selector; the separate
nullable-unwrap arm is left as-is (its own earlier `block_result` branch).

## Slice 2b — the nullable-unwrap arm folds into the ONE dispatch (2026-07-03)

The `tail_is_nullable_unwrap` arm (block_result's last pre-selector Reference path)
carried the `MaterializeView` mechanism INLINE — character-for-character the
`RefDelivery::MaterializeView` dispatch body. Folded: the arm keeps its own entry
(it keys on the DECLARED result + the unwrap tail shape, which the `t == Reference`
arm cannot see) and its body is now `dispatch_reference_delivery(MaterializeView)`
— the #416/#448 mechanism-through-one-dispatch pattern on the Reference side.
Byte-identical on `D-own-1-reference-corpus.loft` (0 diff lines, IR + bytecode +
native Rust), corpus runs clean both backends, suite green.

**Instrument re-read at this point (post-flip, post-fold):** `block_result` is
**320 lines** (was 459), ~24 helper calls (was 45); every VECTOR mechanism flows
through `dispatch_vector_delivery`, every REFERENCE mechanism through
`dispatch_reference_delivery`. Remaining thicket: the TEXT sub-story (the
`text_return` work-buffer promotion + the cross-phase B5-L3/`__ret_text` family —
per-var rules, not a tail-shape selector; a different collapse shape), the
`return_buffer`/`nrvo` plumbing helpers, and the C86 bind-site derivation residual
(expressions.rs `struct_vec_field`).


**Behaviour-PRESERVING.** Corpus `bytecode-comparisons/D-own-1-reference-corpus.loft` (one fn
per Reference-return path: owned-fresh, wrap-call, return-views-local #306, nullable-unwrap,
arg-borrow). Verified: bytecode + native Rust byte-identical before/after (the only introspect
diff was the scope-id label + variable-table columns, proven NON-DETERMINISTIC by re-capturing
the same binary twice). Full suite 2542 green both backends; differential oracle green. No
behaviour change — organisational collapse (the Reference arm was clean; this readies it for
the carried-fact model and shrinks the thicket toward the beacon).
