<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Session note — the store-lifetime sweep, and where #1329 resumes

Branch `tuxedo-stability-impact`, 2026-09-03, tip `86898d42`, `make ci` green.
Written for a cleared context: everything below is measured, and every claim names the
instrument that measured it.

## What landed

Seven issues, each decided by a rule in `formal/` rather than by reading the code and guessing.
The register is now at **one** open issue with no fix (#1329, below).

| # | rule | one-line |
|---|---|---|
| 1318 | `@FR-O-Oracle` | a fn-ref `??` whose argument is a CALL freed the caller's container (`silent-wrong`, sev:high) |
| 1322 | `@FR-O-Borrow` | a `??` default-arm mint was released twice — `_vec_N` and `__vdb_N` name one store |
| 1324 | `@FR-O-Latest` | a reassigned capture leaked, AND an escaping closure read a released store |
| 1325 | `@FR-B-Copy` / `D-op-1` | a `(text, text)` bind MOVED on `--native`; rustc refused what the interpreter ran |
| 1326 | `(L-CapHeap)` + `=` | a rebind of a captured KEYED collection emptied it (`silent-wrong`) |
| 1327 | `@FR-O-Move` → new `(O-Opaque)` | a closure through a fn-TYPED PARAMETER freed the caller's vector (`silent-wrong`, sev:high) |
| 1328 | `@FR-O-NoDiverge` | a fn-ref rebind never freed the displaced store; native aborted at the store ceiling |

Also joined from `../loft`: D-bind-11 (`392694cd`) and D-own-8/D-bind-16 closing #1321/#1323
(`cd10a159`). The join's QUALITY.md optional-audit row was **re-measured**, not reconciled:
`694 | 337 | 5 | 352`, which is neither side's number, exactly as that row's own paragraph says
it will be.

## The one issue still without a fix

**loft#1329 — a `vector` local rebound from a fn-ref call holds one store per iteration.**
Both backends, verified on `origin/main` (b1ccf0e9); at 70 000 iterations the run aborts with
`store table exhausted`. The identical loop over a NAMED function completes.

The cause is read straight off the IR and is not a predicate to widen:

```
x(1):vector<integer>["__ref_1"] = n_mk(i(3), __ref_1(1));    // DIRECT: one buffer, reused
x(1):vector<integer>["__vdb_1"] = fn_ref[0](i(3));           // FN-REF: no buffer passed
```

A direct call is handed a caller-local return buffer and the destination views into it. A fn-ref
call is passed none — the call site allocates its return buffer at RUNTIME (`State::fn_call_ref`)
— so every iteration mints a new store while the destination's dep still names the ORIGINAL
backing local. The non-empty dep is also what makes `owned_ref` (both backends) decline the
displaced free.

**The cure is already decided and is a plan, not a drive-by.** It is the one loft#1183 / #1185 /
#1186 closed on: give the fn-ref call site a CALLER-OWNED heap return buffer, the symmetric twin
of `push_fnref_text_buffers`, with `Data::fnref_text_buffers`' widest-candidate-then-trim shape
for the adaptive ABI, so the call's result is published as a borrow of a buffer the caller frees.
Coordinated parser + interpreter + native. `formal/closures.md` D-clo-12/D-clo-13 carry the
measurement, and the register is explicit that a CLASSIFICATION fix is measured ground and must
not be retried.

Resume from `State::fn_call_ref` (which allocates the buffer today, and is why `fnref_bufs`
tracks it by frame depth), `Parser::push_fnref_text_buffers` as the working precedent, and
`check_argument_geometry`, which will catch an ABI change that gets the slot order wrong.

## Three things that cost real time and will again

**1. Statement context silently retires a guard cell.** `s += g(vs[0])[1]` and
`c = g(pick(vs, 0))[1]` are **correct on the broken build** while the same reads inside an
interpolation are wrong — the accumulate and bind paths reach a different lift decision. Two of
#1318's cells were vacuous until each was scored against the control build. `make falsify`
reports one number for the whole FILE, so it cannot see a vacuous cell: after it says the file
moved, run the guard against `/home/jurjens/.cache/tmp/loft-falsify/<ref>-target/debug/loft`
(symlink the ref worktree's `default/` beside the binary or it cannot find the stdlib) and read
which cells actually fail. Record that census in the `@falsified-at:` note, because the tool's
counts are floors — native stops at its first failing test function.

**2. The exit-leak gate cannot see peak growth, and the store ceiling is what makes it
scorable.** #1328 reports no leak at 1000 iterations on either backend; the whole defect is in
the PEAK. Counting past 65 535 turns a watermark into an accept/reject split, which a guard can
score — and `make falsify` then reports the move on the PANIC channel, not the assert channel,
because no assertion is reached at all.

**3. These parser flags are CUMULATIVE ACROSS PASSES.** #1322's third cure is entirely that
observation: a capture subject reads empty deps on pass 1 and takes one arm, reads non-empty deps
on pass 2 and takes the other, so the variable ends with BOTH flags. Asking `is_skip_free(w)`
asks which arm it ENDED in; asking which arm this pass took answers a different question and
leaked the store in `1248b`. The two cures that failed before it are on the issue.

## Residuals recorded rather than fixed

- **`is_projection_op` is short by `OpGetVectorNullable` / `OpVectorRefNullable`** by its own
  criterion, and adding them strands three records in
  `1040-generic-par-worker-in-generic-fn.loft`: `state/codegen.rs`'s @PLN130 F1 materialise arm
  fires on the deps PROXY while the free sweep reads the ORACLE, and a `par` body's element bind
  sits in the gap — `@FR-O-Proxy`'s hazard in the allocate direction. It does not reproduce on
  `main`, so the doc comment on `projection_ops` carries the measurement instead of an issue.
- **A closure record is released twice** — through the fn-ref value AND its `___clos_N` local, so
  its cascade runs twice. Same rule as #1322, different pair of names, pre-existing. It is the
  shape `tests/redundant_free.rs::the_harness_can_see_a_redundant_free` uses, so fixing it means
  replacing that cell rather than deleting it.
- **What (L-CapHeap) means for a REASSIGNED capture is undecided at the keyed kinds:** a rebind
  outside the closure lets it read the reassigned value at `hash`/`sorted`/`index` and the
  build-time value at vector and struct, because a keyed rebind refills the existing store rather
  than minting. Store lifetime is correct either way; `formal/closures.md` records it as open.

## 2026-09-04 — loft#1336: the owner witness (`(O-Witness)`, D-own-27 opened and closed)

**What landed.** A heap-record local whose assignments MIX ownership (a copy or a minting call
on one, a view on another) now carries a hidden `__own_<name>` in the IR that names the store
it minted while it still holds it; `scopes::owner_witness_locals` picks the locals,
`scan_set` maintains the witness at every `Set` (`witness_set_kind`: release before a mint,
release by `OpDistinctStore` after a view or a mint that reads the local), `get_free_vars`
releases at scope exit, and the local is never-free. Two ops: `OpDistinctStore`, `OpRefAlias`.
Both emitters copy into such a local FRESH and decline the materialise arms for it; native's
private `_own_store_` tracker (the reference route) now serves only hidden temporaries.
`LOFT_NO_OWNER_WITNESS=1` is the A/B; the two `LOFT_NO_JOIN_OWN` controls set it too.

**The filed scope was wrong three ways** — the `reference` field, the copy-bind and the `?` were
each shown not to be the axis (`ownership-history.md` D-own-27 has the cells). The dense twin
was over-freeing at exit on the interpreter, masked by `free_named`'s idempotence.

**Four wrong first cuts, each a measurement** (D-own-27's list): the `= null` entry init is a
stack placeholder; `OpDatabase` consumes the `null_named` placeholder at a first bind; native's
`skip_free → adopt` rule read a witnessed local as a `__ncc_` hoist; loft#778's `k = a[0]; k =
x` materialises off deps a LATER copy stripped.

**Guard-authoring trap:** a `reference<T>?` field pointing at a callee's local dangles at its
return — five cells of the first guard reported use-after-free from the test's own chain
helper. Build the chain in the frame that walks it.

**Filed apart:** loft#1337 — a view of a local returned through a `-> S?` return is not
materialised (the dense return leg copies into its buffer; the nullable one has no buffer).
