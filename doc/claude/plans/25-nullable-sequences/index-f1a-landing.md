<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN25 — INDEX (`v[i]`) FLIP: the F1a landing phase (START HERE next session)

Cold-start entry point for landing the index (`v[i]` → `τ?`) flip as the default. The
**foundation is already built and committed** (dev-gated, inert); this doc is the step-by-step
to flip it ON without breaking the tree. Read [RESUME.md § MOST RECENT](RESUME.md) first for the
one-paragraph context, then this.

## State (what's already done, on `tuxedo-pln85-fuzz-proof-gate`, pushed)

- **Mechanism (`c0aad2b2`, `c58d7623`):** `parse_vector_index` (fields.rs) sets `self.last_index_fit`
  for each SCALAR `v[i]` read; `parse_index` (fields.rs ~656) wraps the element type `Optional`
  when `!last_index_fit`, **gated behind `LOFT_INDEX_DEV`** (temporary dev-gate; folds into
  `pln25_dn1_enabled` at the end).
- **Fit-proofs (`index_provably_fit`, fields.rs)** — a `v[i]` types NON-null when the index is:
  - any compile-time constant (`self.const_int(index).is_some()` — covers `v[0]`, `v[-1]`, `v[2+3]`);
  - a for-loop iter var (`self.vars.is_active_loop_var(v)` — no separate stack);
  - proven `< len(vec)` by an enclosing `if idx < len(vec)` guard (`index_bounded: Vec<(u16, VecKey)>`,
    fed in `parse_if` (control.rs) by REUSING `operators::collect_guard_pairs`, THEN-only).
- **Reject mechanism proven:** undefended general-var index into a non-null store rejects with the
  standard `(N-Store)` diagnostic. Accept path (const / iter / guard / `?? d`) all run.

## The blocker that stopped the naive "flip + migrate 8 sites" (READ THIS)

Flipping `LOFT_INDEX_DEV` ON breaks **6 wrap tests**, and the reject-count measurement MISSED most
of them (see [[measure-flip-by-running-suite]] — measure by RUNNING the suite, not grepping rejects):

1. **SILENT copy-elision wrong-answer (the load-bearing fix, do FIRST).** `v[i]` typing `Item?`
   mis-classifies a MUTATED / ESCAPING element borrower (`e = v[i]; e.x = 99`) in the copy-elision
   analysis, so the copy-keep decision flips and the mutation LEAKS to the source. Repro:
   `tests/scripts/85-borrow-elision-element-borrower.loft` `mut_elem` returns the wrong value on
   BOTH backends (a wrong ANSWER, no compile error). This is the deps subsystem — loft's #1 weakness.
2. **loft-in-loft COMPILER ripple:** `dir` / `last` / `parser_debug` / `wasm_dir` — these run the
   compiler over other files, and `lib/parser.loft` + `lib/code.loft` use `v[i]` (~130 undefended
   reads; 0 direct rejects, but they ripple through these tests). Same class as the DN1 flip's F1a.
3. **`library_suite` (audience_crystal)** — a library `v[i]` / field-defense interaction.
4. **`loft_suite`** — the .loft corpus (85-borrow + the direct rejects below).

## The steps (each ends green; commit per step)

**Step 0 — reproduce + MEASURE by running (not grepping).** Flip the gate in fields.rs
(`std::env::var_os("LOFT_INDEX_DEV").is_some() && …` → `crate::keys::pln25_dn1_enabled() && …`),
build, then run and read FAILURES (asserts / panics / wrong values), both backends:
`cargo test --release --test wrap --test issues` and `LOFT_TIMEOUT=90 loft --native <corpus>`.
Then REVERT the gate flip and work under `LOFT_INDEX_DEV=1` per step. (Keep the gate dev-gated until
Step 6 so the tree stays green between commits.)

**Step 1 — ✅ DONE (copy-elision `Optional`-peel).** Fix = peel `.base()` before reading the
borrower's deps in `use_analysis` (the `borrowers` filter, `analyze_fn` ~504): `function.tp(e)
.base().depend().contains(&v)`. `Type::depend()` has no `Optional` arm (it recurses only through
`RefVar`/`Tuple`), and the index flip attaches the element's dep to the INNER type before the
`Optional` wrap (`parse_index` fields.rs 632-676 → `Optional(Reference(Item, [xs]))`), so without
the peel a nullable element borrower's dep was invisible → a mutated/escaping `e = v[i]` was missed,
the copy wrongly elided, and the mutation LEAKED to the source (SILENT wrong-answer).
- **Why detection-only is the whole fix (no re-point change needed):** the peel makes a mutated/
  escaping borrower `∈ ineligible` → the plan is REFUSED → copy kept (the fix). A read-only borrower
  is now detected + planned, but the re-point (`make_independent`/`depending`, also `Optional`-opaque)
  no-ops — which is BENIGN: `elide_rewrite` inlines the source into the borrower's def (`e = g.c[i]`),
  reads use that stored ref (not `e`'s dep), and the free pass already peels `Optional`. Proven on the
  boundary matrix (read-only elides+reads right / mutated keeps copy / escaping keeps copy) + a
  read-only stress set (late read, two borrowers, into-call), value+leak, BOTH backends, both gates.
- **Safe-by-construction for the default suite:** `.base()` is identity for non-`Optional` types;
  for an `Optional` borrower it can only ADD a detection, which only ever REFUSES a plan (keeps a
  copy — always correct) or adds a harmless read-only detection. It cannot introduce a wrong answer.
- **Regression:** `tests/scripts/25-index-elision-borrower.loft` (self-asserting; gate-OFF
  byte-identical) + `tests/pln25_dn1_consumption.rs::index_dev_elision_borrower_{interpret,native}`
  (runs it under `LOFT_INDEX_DEV=1`, both backends). NOT touched: `depend()`/`depending()`/`deps_ref()`
  globally (74/31/3 callers, DN1-default-live — wider than the failing region, no correctness gain).

<details><summary>Original Step-1 notes (starting points)</summary>

Make the element-borrower classification peel `Optional` so `e = v[i]` where `e: Item?` is still
recognized as borrowing `v`, and its mutation/escape is still detected (so a mutated/escaping
borrower KEEPS the copy). Starting points found this session:
- `src/use_analysis.rs` — `ElidePlan` (struct ~195; `borrowers` field ~202 "Vars that BORROW `v`
  (`e = v[i]` …)"; `visit` ~210; the re-point comment ~498). The plan is only emitted when "every
  borrower is read-only and non-escaping" — that read-only/escaping classification is what breaks
  when `e` is `Optional`-typed.
- `src/scopes.rs:301` — "A var that BORROWS `v` (`e = v[i]`, `deps` ∋ `v`)".
- **Method:** boundary matrix on the 85-borrow trio (read-only ELIDES + reads right; mutated KEEPS
  COPY + source unchanged; escaping KEEPS COPY) — value + leak, BOTH backends (the
  `engineering-rigor` + `loft-codegen` skills; the deps north-star is `OWNERSHIP_MODEL.md`). The
  fix is almost certainly a `.base()`/`peel_optional` at the point the classifier reads the
  borrower's type or matches the `e = v[i]` shape — but PROVE it on the matrix, don't guess (this
  is the deps system; symptom-patching it is how the heap model became loft's #1 weakness).

</details>

> **🟠 BRANCH WAS RED (pre-existing, NOT caused by Step 1) — surfaced 2026-07-01b by a full-suite run.**
> Mid-step-f debt the RESUME's targeted "issues/wrap/format green" claims did not cover. **Run via
> `find_problems`, NOT raw `cargo nextest`** — the wrapper rebuilds cdylibs + clears stale `.loftc`; a
> raw nextest shows multiplayer/viewer/html_asyncify/runtime_warnings as STALE-FIXTURE FALSE failures.
> The authoritative pre-fix set was **17 real failures**, none copy-elision:
> 1. **✅ FIXED (`73fe4f6c`) — store codec now serializes `Type::Optional` (`TyOptional` variant, disc 25).**
>    Cleared all 11 store-codec failures (`ir_read::*round_trip*`, `corpus_store_codec_round_trips`,
>    `g2_ir_check`). Root: `ir_store::write_type` peeled the wrapper (persisted `Optional(Integer)` as
>    `Integer`); read had no arm. Note: a full `extract.py` regen is currently unsafe (the binary's
>    `--show-rust` now emits `db.dbref()` for struct-ref fields, which extract.py drops + would shift
>    offsets) — the `TyOptional` schema line is hand-added, `ir.loft` is the source of truth.
> 2. **REMAINING — 6 un-migrated DN1/DN3 Rust tests** (step-f cleanup, all test migrations — NO new real
>    bug): `expressions::{call_int_null, call_text_null}` (return null from a non-null fn → `-> τ?`);
>    `expressions::{bounded_mixed_type_operator, bounded_unary_operator}` (`/`,`%` variable divisor →
>    defend or `-> τ?`); `exit_codes::div_by_literal_constant_no_warning` (the constant-divisor fit-proof
>    already WORKS — `divisor_provably_nonzero`; the failing part is a VARIABLE divisor `c / y` into
>    `-> integer` that DN3 now correctly REJECTS vs the old warning → carries the design call of whether
>    the runtime div-warning is retired under DN3, like the index warning); `error_messages::baselines_are_locked_in`
>    (golden refresh).
> 3. **Pre-existing clippy warning** — `operators.rs:1716 handle_operator` (8/7 args, DN3-division).
>
> Recommend: the 6 migrations (one carries the div-warning-retirement call) + clippy, then continue the index flip.

**Step 2 — lib compiler migration (`dir`/`last`/`parser_debug`/`wasm_dir`).** Under `LOFT_INDEX_DEV=1`,
find the `v[i]`-into-non-null sites in `lib/parser.loft` + `lib/code.loft` that reject (or that a
compiler test surfaces), and defend each: `?? default`, an `if idx < len(v)` guard, or a `τ?`
declaration. Mirror the DN1 F1a lib-lexer sweep (`d620d77a`) — most were `-> τ?` on token accessors.

**Step 3 — audience_crystal library** (`lib/audience_crystal/…`) — the `library_suite` failure. Same
treatment; check whether it's a real `v[i]` reject or the field-defense warning escalating.

**Step 4 — the direct N-Store rejects.** `85-borrow` `esc_elem` (returns the element itself → honest
`-> Item?`, BUT only after Step 1, since the `Optional`-heap-return interacts with elision — the
naive `-> Item?` broke `mut_elem` before Step 1); issues `p124` `arr[idx]` ×2 (→ `-> float?` +
adapt callers), `p155`, `p170` (`v[len-1]` computed index), `p379`. These change test SEMANTICS —
migrate with care, not a mechanical `?`.

**Step 5 — len-capture proof, IF Step 2/4 surfaces it.** `n = len(v); if idx < n { v[idx] }`. The
extractor already supports it: `collect_guard_pairs` takes a `captures: &HashMap<u16, VecKey>` — at
parse time, populate it by tracking `Set(local, Call(len,[vec]))` (mirror the warning walk's
`len_captures`), and pass it instead of the empty map in `parse_if`.

**Step 6 — flip the gate + retire the warning + graduate.** Change the fields.rs gate
`LOFT_INDEX_DEV` → `pln25_dn1_enabled`; RETIRE the now-redundant `FaultKind::VectorIndex` /
`TextIndex` warning under DN1 in `emit_undefended_warning` (operators.rs — same as Div/Rem got in
`7042d94c`, since the TYPE is now the enforcement); graduate `tests/scripts/25-index-nullable.loft`
(accept: const / iter-var / then-guard / else-guard / `??` / honest `τ?`) + a reject twin in
`102-expected-errors.loft`. Full suite green both backends → done; update RESUME F1b + the
formal/types.md deviation.

## Also on the DN3 fault-op docket (separate slices, not part of F1a)

- **gap #3** — `??` discharges regardless of its fallback's nullability (`handle_null_coalesce`,
  operators.rs:1366 `*ctp = ctp.base()`); `x ?? null` unsoundly types non-null. Rule: result is `τ?`
  iff the fallback is nullable. Own blast radius (every `a ?? nullableVar`).
- **call-arg N-Store** — a nullable passed into a non-null CALL ARG is not checked (`takes(v[j])`
  doesn't reject). Affects division equally; closing it covers all fault-ops at once.
