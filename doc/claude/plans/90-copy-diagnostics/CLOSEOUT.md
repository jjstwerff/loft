<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN90 close-out — concrete steps to finish the plan

The verified remainder, sequenced, with per-item build steps (file · fn · line), effort,
risk, dependencies, and exit criteria. Produced 2026-07-06 by re-verifying every open item
against the live tree (not the docs) — see the "verified state" line on each. Companion:
[REMAINING.md](REMAINING.md) (the prioritised release-gate view) · [README.md](README.md) (status) ·
[borrow-return/DESIGN.md](borrow-return/DESIGN.md) (the A1b/A2 mechanism, grounded in captured IR).

## What is already done (do not rebuild)

- **Phase B move-elision** — DEFAULT ON, merged (#514, squash `46ecd3dc`). All four copy
  shapes, flat + nested. `LOFT_NO_MOVE_ELIDE` opts out.
- **Phase A `--report-copies` report** — merged (#510). `report_copies`
  (`src/use_analysis.rs:1772`), the `--report-copies` CLI flag (`src/main.rs:3810`), the
  `CopyClass` 4-way bucket (`:42`), and the **bound-vs-unbound survival split**
  (`survival_class`, `:855`, gated on `report_copies_enabled()`) are BUILT. Only the
  *enforced* (default-on `Level::Warning`) channel is missing — that is W5 below, **not** a
  rebuild.
- **#462** — CLOSED (native record leak, subsumed by the @PLN85 store/adopt-free siblings;
  compact repros verified clean both backends). Off the list; re-check only if a crawler-scale
  record leak resurfaces (no in-repo corpus to test it now).

## loft2 division-of-labor check (done 2026-07-06)

loft2 (`../loft2`, branch `tuxedo-fix-511`) carries **no fix on any @PLN90-remainder surface**
(its diff vs `origin/main` on `scopes.rs`, `parser/control.rs`, `generation/dispatch.rs`,
`generation/ops/parallel.rs`, `use_analysis.rs` is empty). Its active work is @PLN93
(collection-capture-into-closures, "borrow by shared DbRef", issue #511) + a branch-local #513
store-persist fix. **Zero duplication risk.** One adjacency to honour: @PLN93's capture uses the
same **Reference-DbRef borrow chokepoint** that W1's caller-side materialise touches — when W1
lands, re-check it against @PLN93's captured-collection borrows (and loft2 must rebase onto main
to pick up #514).

---

## Work items

### W1 — A1b: temporary-subject borrow-return UAF  ·  P0 · **M · HIGH risk** · the release blocker

**Verified state:** OPEN. [borrow-return/cell-escape-temp.loft](borrow-return/cell-escape-temp.loft)
fails **both** backends — interp assertion-fails (`len(a)=0`), native panics — loud under
`LOFT_POISON`. A1 (`f70a729d`) fixed the interp `gen_if` derail but did **not** close the
lifetime hole; `--native` carries it latently.

**Root (DESIGN.md §A1b, captured):** the `one_buffer_chain` NRVO optimisation reuses ONE store
(`__ref_1`) as **both** the `Filled` subject **and** the return buffer. `g` returns
`__ref_1.items` — a borrow of that very store — and the materialise's `OpDatabase(_dst=__ref_1,64)`
**reallocates the store the borrowed `_src` points into, freeing the source before the copy**.
The native materialise gate (`dispatch.rs`, `if _src.store == _dst.store { alias } else { copy }`)
is downstream of the wrong buffer choice — the root fix is the buffer, not the alias/copy test.

**Target IR (PROVEN clean, both backends + POISON — [cell-escape-temp-FIXED.loft](borrow-return/cell-escape-temp-FIXED.loft)):**
materialise the borrow into a store DISTINCT from the subject, free the subject AFTER the copy —
`r = n_g(c,…); OpClearVector(out); OpAppendVector(out, r); OpFreeRef(c); return out`.

**Localized to the verdict level (2026-07-06 — instrumented, reverted).** `LOFT_TRACE_RR` on
`cell-escape-temp`'s `n_h` shows TWO work-refs: `__ref_1` → `Rename{buf_attr:1}` (becomes
`__retbuf`) and `__ref_2` (the temp `Filled` subject) → `Bind{buf_var:6, substitute:true}` — the
`substitute_work_ref` merges the subject store into the buffer store. **That merge is the collapse.**
cell-escape (safe) has ONE work-ref (subject is a param, not a work-ref) — so the UAF axis is "the
borrowed subject is a temporary the fn itself constructs → a subject work-ref that gets
`Bind{substitute}`." Falsified en route: a single "suppress the Rename" guard is **insufficient**,
and `g.returned()` deps name the hidden buffer (attr 1), NOT the visible `e` (attr 0) —
`filter_hidden` strips the borrow; the "buffer borrows a visible param" fact must be read from g's
BODY. Full trace in [borrow-return/DESIGN.md § A1b gate-2](borrow-return/DESIGN.md).

**Chokepoint localized to the CONSTRUCT-STORE ALLOCATION (gate-4, 2026-07-06).** A gated
delivery-level fix (`LOFT_A1B`, `tail_call_borrows_temp_subject` → `CopyBorrow`) was built, tested,
and **reverted as insufficient**: the predicate fired correctly but the UAF persisted because the
inline subject construct is built with the fn's `__retbuf` work-ref (`__ref_1`) as its store —
`OpDatabase(__ref_1, 67)` inside the `Object` block, where `__ref_1` IS `n_h`'s `__retbuf`. The
`Filled` subject and the return buffer are the **same store before delivery runs**; the
`one_buffer_chain` optimisation collapses all three roles (subject / g-buffer / `__retbuf`) onto one
`__ref_N`. Full trace + reverted-IR: [borrow-return/DESIGN.md § A1b gate-4](borrow-return/DESIGN.md).

**Build steps (the remaining work — upstream store split + delivery materialise):**
1. Find where the inline heap construct passed as a call arg is allocated the fn's `__retbuf`
   work-ref (`src/parser/objects.rs` construct lowering / the call-arg lifting / `one_buffer_chain`
   in `chain_site_set_shape`). Give a **borrowed** heap-subject construct a **distinct** work-ref, so
   the subject store ≠ `__retbuf` and ≠ g's scratch buffer (three distinct stores).
2. With the subject distinct, the delivery-level materialise (gate-3 plan: route to `CopyBorrow` /
   `materialize_vector_return_into`, copy g's borrowed result into the owned `__retbuf`, free the
   subject after) becomes correct — the two halves compose. The reverted `tail_call_borrows_temp_subject`
   predicate + `LOFT_A1B` gate are validated and cheap to reconstruct.
3. Confirm the native materialise gate (`src/generation/dispatch.rs`) hits the copy branch with a
   distinct store (no double-alloc).
4. **Gate** behind `LOFT_A1B` / `LOFT_JOIN_OWN`; suite byte-identical gate-off.
5. Walk the boundary matrix (named / temp / escape × value + length + leak × `LOFT_POISON` × both
   backends), then the full suite gate-on + the @PLN89 differential oracle.
6. **@PLN93 interaction check** (see loft2 note) before flipping.
7. Promote `cell-escape-temp.loft` to a regression guard under `tests/scripts/`
   (e.g. `85-temp-subject-borrow-return-uaf.loft`).

**Depends on:** nothing (A1 already landed). **Blocks:** W2, W3, W4, W5.
**Exit:** all 6 matrix cells + `cell-escape-temp` clean both backends under `LOFT_POISON`; suite
byte-identical gate-off, green gate-on; oracle green.

### W2 — A2: struct-field `b.rows` copy→alias  ·  P1-opt · M · **depends on W1**

**Verified state:** OPEN. [borrow-return/br_field.loft](borrow-return/br_field.loft)
(`fn f(b: Box) -> vector<E> { b.rows }`) still **copies** into `__retbuf` on both backends
(`OpClearVector(__retbuf); OpAppendVector(__retbuf, OpGetField(b,0,66))`).

**Build steps:** route the struct-field-read tail — `classify_vector_delivery`
(`src/parser/control.rs:1043`) returns `Delivery::CopyBorrow` at `:1109`, gated by
`tail_is_struct_field_read` (`:1082`), dispatching to `copy_borrow_tail_into_retbuf` (`:5388`) —
to the **alias-return** path (`Delivery::Rename` → `ref_return`, the path the A1-fixed match arm
already uses), plus the callee-ABI change (drop the `__retbuf` fill so the alias returns as a
plain `DbRef`; DESIGN slice 1 / option (c)).

**Depends on:** **W1 (hard).** A bare alias of a *temporary* subject dangles (F1) — A2 needs W1's
caller-side materialise in place first. **Exit:** `br_field` returns the alias (`OpGetField`, no
`OpAppendVector`) both backends; matrix cells clean; suite gate-off byte-identical.

### W3 — Wasted empty-buffer alloc  ·  P1-opt · S · rides W2's ABI

**Verified state:** CONFIRMED. A borrowed-alias return still takes + `OpClearVector`s a
`__retbuf` it discards. Removing it **is** the slice-1 ABI change (borrowed-view returns drop the
`__retbuf` param), so it lands with/after W2 — correct-as-is today.
**Exit:** borrowed-view fn signatures carry no `__retbuf` param; no clear of a discarded buffer.

### W4 — O-Complete: analysis totality  ·  P0-umbrella · L · spans W1

**Verified state:** not proven **total** — the umbrella the borrow-return items live under
("an incomplete fact is a miscompile or a leak"). Closing =
(a) **W1 supplies the one missing borrow-return representation** (temp-subject vector-local);
(b) **eliminate the two non-total fallbacks** in the ownership analysis — the `_ => Owned`
catch-all default and the `r = x` value-vs-bind copy gap (memory: `pln85-over-free-match-return`);
(c) the **@PLN89 differential oracle + fuzz gate green** across the borrow-return corpus.
**Exit:** no catch-all `Owned` default reachable on the borrow-return paths; oracle + fuzz green;
STABILITY_ROADMAP `return-bind-ownership` row → CLOSED.

### W5 — Enforced copy lint (the plan's namesake)  ·  Phase 2 · S–M · **after W1+W2**

**Verified state:** the **report is built** (#510 — `report_copies`, `--report-copies`,
`survival_class`, `CopyClass`, `MAT-WORKLIST`). MISSING = the **enforced channel**: route the
report through the existing `Level::Warning` diagnostics path (`src/data.rs` `diags.add_at(...)`,
rendered by `src/diagnostic_render.rs`) so **Avoidable** copies fire as default warnings; resolve
`VerdictRow.loc` to real source spans (the location gap); tests + doc.

**Build steps:** (1) add a `warn_copies_enabled()` gate in `src/keys.rs` (default OFF → promote to
default-on once the Avoidable set is drained); (2) for each **Avoidable** row emit
`diags.add_at(Level::Warning, "copy", file, line, col, <msg + &/restructure hint>)`; **Forced** →
informational, **Implicit/Eliminated** → silent; (3) resolve `VerdictRow.loc` to real spans;
(4) tests in `tests/use_analysis.rs` + a `--report-copies` golden; (5) doc.

**Depends on:** **W1 + W2 must land first** — `field_return` / `b.rows` are currently *Avoidable*;
until they auto-elide the lint over-warns on copies the compiler is about to stop making (the
Avoidable set must drain before the warning goes default-on). **Exit:** a `Level::Warning` per
Avoidable copy with source loc + hint; corpus warning-count matches the drained Avoidable set
(near-zero after W1/W2); lowering byte-identical.

### W6 — par-dispatch native E0308  ·  P0 · S · **✅ DONE (2026-07-06)**

**Landed:** `tuple_arg_prep` (`src/generation/ops/parallel.rs`) gained a `Type::Function` arm —
reads the `i32` fn-index at offset 0 of the element and pairs it with `DbRef::NULL`
(vector-stored fn-refs are non-capturing), yielding the `(u32, DbRef)` tuple the worker expects.
Note refined during the fix: **`par` compiles its worker to native under *both* backends**, so the
E0308 blocked the interpret par-path too (not native-only). Guard:
`tests/scripts/507-par-vector-fnref.loft` (single + multi fn-refs, runs under both backends);
native_scripts + wrap `loft_suite` + the three `p4d_a2` tests green.

**Verified state (before fix):** native emitted a bare `DbRef` where `(u32, DbRef)` is expected
(`error[E0308]`): `tuple_arg_prep` had **no `Type::Function` arm**, so a `vector<fn-ref>` element
fell through to `("", "elm")`.
**Build steps:** add a `Type::Function(_,_,_)` arm to `tuple_arg_prep` that builds the fn-ref
tuple from the element record, mirroring the working for-loop unpack
(`tests/generated/issues_p4d_a2_vector_fn_ref_for_loop.rs:334`) — read the fn-index `i32` at
offset 0 of `elm`, emit `(idx as u32, DbRef::NULL)` (vector-stored fn-refs are non-capturing), pass
`_p`.
**Depends on:** nothing. **Exit:** `par_fnref.loft` compiles + runs `--native`; un-ignore /
un-timeout the native side of `p4d_a2` (`tests/issues.rs:11806`).

### W7 — Phase 3: explicit copy-intent syntax  ·  Phase 3 · S design + M · **after W5**

**Verified state:** NOT STARTED (design only — COPY_DIAGNOSTICS.md:234-244, :332-334: "the inverse
of `&` — opt into an independent copy, silence the report at *that* site; sparse/per-site, never a
global `allow`").
**Build steps (mirror the `&` machinery):** proposal — a **prefix keyword `copy <expr>`** (the
exact mirror of `&`; a method `.copy()` collides with user methods). Add `"copy"` to `KEYWORDS`
(`src/lexer.rs:138`); detect it beside `&` in `parse_operators` (`src/parser/operators.rs:493-497`),
setting a one-shot `copy_pending` flag (twin of `amp_pending`, declared `src/parser/mod.rs:184`); at
the bind-site decision (`src/parser/expressions.rs:~1236`) force a materialise even where the
default borrows; stamp the copy's `VerdictRow.class` as intended (Implicit) so **W5's lint stays
silent** there.
**Depends on:** **W5** (its *silencing* half has nothing to silence until the lint fires).
**Exit:** `copy expr` forces an independent copy both backends; the lint is silent at that site; tests.

### W8 — Empty-arm parse normalise  ·  P1-robustness · S–M · **RE-SEQUENCED: after W1/W2**

**Verified state (2026-07-06, introspected — deeper than filed):** the divergence is **not**
cosmetic. Single-line `_ => { [] }` → return buffer `_mvcopy_1`, `jo_arm_copy` delivery,
`return if … else ;` (value-if, Null else). Multi-line → return buffer `__retbuf` **plus** an
extra `__vdb_1` scratch ref, the `if` demoted to a bare statement, then
`OpFreeRef(__vdb_1); return null` — the trailing `[]` in a block is parsed as a discarded
statement + a separate null value-return. Both still run clean both backends (the repro only
exercises the empty arm), so it stays robustness, not correctness.
**Why re-sequenced:** the two formattings pick **different delivery paths** (`_mvcopy_1`/jo_arm_copy
vs `__retbuf`/materialise) — the exact return-buffer-selection machinery W1/W2 rewrite. Normalising
it first would churn the code the blocker touches (violates "no simplify during debug"). Do it
**after** W1/W2, once the borrow-return delivery is unified, so both formattings converge on the
settled arm-value delivery.
**Build steps:** at parse, classify a block whose trailing expression is the arm value the same as
the single-line arm-value form (deliver the trailing expr, not "statements + null return"); then
both lower to one canonical IR. **Depends on:** W1, W2 (delivery unification). **Exit:** both
formattings produce identical IR; suite green.

### W9 — Drain bucket 2 (grow the auto-elision set)  ·  north-star · L incremental · FOLLOW-ON, not a close gate

Extend the `Borrow`→`ElidePlan` engine to more Avoidable copies (var-buffer cases the analysis
can't yet prove; construction where the source provably out-lives a non-escaping record). Each
Avoidable row surfaced by W5 is a candidate. This is the **perpetual north-star worklist** — it
does **not** gate closing @PLN90 (else the plan never closes); track it as ongoing after close.

---

## Execution order & dependency graph

```
independent, start now ─┬─ W6  par-native E0308            (S, P0)
                        └─ W8  empty-arm parse normalise   (S–M, robustness)

borrow-ABI critical path ── W1  A1b UAF  (M, HIGH, BLOCKER)
                              ├─→ W2  A2 field→alias  (M) ─→ W3 wasted-buffer (S)
                              └─→ W4  O-Complete  (L)  [W1 + remove 2 fallbacks + oracle/fuzz]

lint thread (after W1+W2 drain Avoidable) ── W5  enforced lint (S–M) ─→ W7  Phase 3 copy-syntax (M)

follow-on (does NOT gate close) ── W9  drain bucket 2  (L, ongoing)
```

**Recommended sequence:** W6 + W8 first (cheap, independent, one is a P0) → **W1** (the blocker,
sequenced with care — #1-weakness machinery, gated + full-matrix + oracle) → W2 → W3, with W4
closing as W1 lands and the two fallbacks are removed → W5 → W7.

## Close criteria for @PLN90

The plan closes when: **P0** — W1 (A1b) closed ⇒ W4 (O-Complete) closed, and W6 (par-native)
closed (#462 already closed); **namesake** — W5 (enforced lint) shipped; **design complete** — W7
(Phase 3); **robustness** — W3, W8. **W9 (drain bucket 2) is explicitly a follow-on north-star, not
a close gate.**

## Effort roll-up

| item | effort | risk | gates close? |
|---|---|---|---|
| W1 A1b UAF | M | **HIGH** | yes (P0 blocker) |
| W2 A2 field→alias | M | med | yes (drains Avoidable for W5) |
| W3 wasted-buffer | S | low | robustness (folds into S2.3) |
| W4 O-Complete | L | med | yes (P0 umbrella) |
| W5 enforced lint | S–M | med (over-warn until W1/W2) | yes (namesake) |
| W6 par-native | S | low | yes (P0) |
| W7 Phase 3 syntax | S+M | low | yes (design complete) |
| W8 empty-arm parse | S–M | low | robustness |
| W9 drain bucket 2 | L | — | no (follow-on) |

---

## Landable increments (small steps, each independently ship-able)

Each `S…` step is one landable slice — a few hours, its own verify boundary, commit + push
when green. The method (memory `analysis-first-instrument-gated`): **instrument the fact →
build it inert → apply gated → matrix both backends → suite → guard test.** Start with the two
independents (W6, W8), then W1.

**W6 — par-native (do first: P0, independent, ~S):**
- `S6.1` Check in the failing native repro (`par_fnref.loft` → `tests/scripts/`); confirm
  `--native` E0308, `--interpret` ok. *(instrument — prove the harness fails)*
- `S6.2` Read the working for-loop unpack (`tests/generated/issues_p4d_a2_vector_fn_ref_for_loop.rs:334`)
  to capture the exact tuple-build codegen. *(capture the answer)*
- `S6.3` Add the `Type::Function(_,_,_)` arm to `tuple_arg_prep`
  (`src/generation/ops/parallel.rs:157-211`): offset-0 `i32` → `(idx as u32, DbRef::NULL)`. *(apply)*
- `S6.4` Rebuild native; repro passes both backends; un-ignore/un-timeout `p4d_a2`
  (`tests/issues.rs:11806`). *(verify + guard)*
- `S6.5` `native_scripts` + `issues` green; commit + push. *(ship)*

**W8 — empty-arm parse normalise (independent, ~S–M):**
- `S8.1` `introspect` both formattings; pin the two IRs in a golden. *(instrument)*
- `S8.2` Locate the empty match/if-arm parse; choose the canonical form (empty-block). *(analysis)*
- `S8.3` Normalise both formattings to the empty-block IR. *(apply)*
- `S8.4` Re-introspect → identical IR; suite green; guard cell carrying both formattings. *(verify)*

**W1 — A1b UAF (the blocker — instrument-first, ~M, HIGH risk):**
- `S1.1` Promote `cell-escape-temp.loft` to a checked-in POISON matrix (named/temp/escape ×
  value+len+leak); confirm temp/escape fail loud both backends under `LOFT_POISON`. *(instrument — prove failure)*
- `S1.2` Hand-write + prove the clean separate-buffer materialise IR for the `temp` cell against
  the live tree (re-verify `A1b-TARGET-escape-temp.txt`). *(capture-and-diff)*
- `S1.3` Add the suppression predicate in `chain_site_set_shape` (callee returns borrow-of-param
  AND that param's arg == reuse var `w`) as a **log-only** boolean (env-gated); verify it fires
  ONLY on temp/escape, nowhere else in the suite. *(build the fact inert)*
- `S1.4` Behind `LOFT_JOIN_OWN`, on suppression: keep `w`, allocate a separate `out`, emit
  `OpClearVector(out); OpAppendVector(out, call)`, move `OpFreeRef(w)` after the append. *(apply — interp)*
- `S1.5` Confirm the native materialise gate (`dispatch.rs`) hits the distinct-store copy branch;
  no double-alloc. *(apply — native)*
- `S1.6` Matrix: 6 cells value+len+leak both backends under `LOFT_POISON`. *(verify)*
- `S1.7` Suite gate-on (issues/leak/native/wrap/native_scripts) + @PLN89 oracle; byte-identical
  gate-off. *(verify)*
- `S1.8` @PLN93 interaction check; guard `tests/scripts/85-temp-subject-borrow-return-uaf.loft`;
  flip default-on / fold the gate. *(ship)*

**W2 — A2 field→alias (after W1, ~M):**
- `S2.1` `introspect br_field.loft`; pin the current copy IR both backends. *(instrument)*
- `S2.2` Route `tail_is_struct_field_read` in `classify_vector_delivery` (`control.rs:1082/1109`)
  → `Delivery::Rename` (alias) instead of `CopyBorrow`, gated. *(apply)*
- `S2.3` Drop the `__retbuf` fill for the borrowed-view return (option (c)) — **this is W3**. *(apply)*
- `S2.4` Matrix (subject-outlives/temp/escape) — W1's materialise catches the temp case; both
  backends. *(verify)*
- `S2.5` Suite + guard; flip. *(ship)*

**W4 — O-Complete (spans W1; fallback-removal steps, ~L):**
- `S4.1` Enumerate reachable `_ => Owned` catch-all sites in `ownership_of`; debug-assert/log when
  hit on a borrow-return path. *(instrument)*
- `S4.2` Replace each reachable catch-all with an explicit typed classification; close the
  `r = x` value-vs-bind gap. *(apply)*
- `S4.3` @PLN89 oracle + fuzz gate green across the borrow-return corpus. *(verify)*
- `S4.4` STABILITY_ROADMAP `return-bind-ownership` → CLOSED. *(ship)*

**W5 — enforced lint (after W1+W2, ~S–M):**
- `S5.1` Add `warn_copies_enabled()` in `keys.rs`, default OFF. *(scaffold)*
- `S5.2` Resolve `VerdictRow.loc` to real source spans (the location gap). *(build the fact)*
- `S5.3` For each Avoidable row → `diags.add_at(Level::Warning, "copy", loc, msg+hint)`; Forced =
  info; Implicit/Eliminated = silent. *(apply)*
- `S5.4` Golden tests (`--report-copies` + warning output); corpus warning-count = Avoidable set. *(verify)*
- `S5.5` Once Avoidable near-zero (post W1/W2), promote `warn_copies_enabled()` default-on; doc. *(ship)*

**W7 — Phase 3 copy-syntax (after W5, ~S+M):**
- `S7.1` Add `"copy"` to `KEYWORDS` (`lexer.rs:138`) + a lexer test. *(scaffold)*
- `S7.2` Detect `copy` beside `&` in `parse_operators` (`operators.rs:493-497`); set `copy_pending`
  (twin of `amp_pending`, `mod.rs:184`). *(parse)*
- `S7.3` At the bind-site (`expressions.rs:~1236`), `copy_pending` forces a materialise even where
  default borrows. *(apply)*
- `S7.4` Stamp `VerdictRow.class = Implicit` at a `copy`-site so W5's lint is silent there. *(integrate)*
- `S7.5` Tests both backends: forces an independent copy; lint silent at that site. *(verify)*
