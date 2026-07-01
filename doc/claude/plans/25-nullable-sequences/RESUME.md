<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN25 dense-default — RESUME HERE (cold-start handoff)

Single resume point for the dense-default value-model rewrite. Written so a fresh
session can pick up after a `/clear`. **Read order:** this file →
[full-design.md](full-design.md) (the consolidated design) →
[storage-vs-access-nullability.md](storage-vs-access-nullability.md) (the invariant +
probe verdicts) → [implementation-steps.md](implementation-steps.md) (the phase order) →
[`../../formal/types.md` § Nullability](../../formal/types.md) (the `N-*` rules).

---

## TL;DR — where we are (updated 2026-06-30)

- **`lima-default-borrow-elision` is MERGED to `main`** (via #467/#468/#469 etc.) and the
  branch is deleted. Its scalars Phase-0 + DN4 + N-Arith work is now ON `main`.
- **@PLN25 now continues on `tuxedo-pln85-fuzz-proof-gate`** (the single live branch, off
  `main`, pushed, no PR). Step 1 (`Type::Optional`) landed here (`d121f94c`).
- **Suite green** — `find_problems.sh` 0 failures on both backends after Step 1.
- **The vectors half is DONE and on `main`.** `vector<S>` is dense (`main_vector<S>`,
  no `__nullable`); `vector<S?>` is the nullable opt-in; `v[1] == null` is true; the
  canonical incoherence probe is coherent on both backends. The #465 borrowed-view
  over-free is fixed. Merged via `#412` (gate flip + keyed-dense), `#467` (dense
  vectors + copy-vs-borrow elision, vectors-green checkpoint), `#468` (borrow elision
  default-on + Tier-1.5).
- **The scalars + TIGHTEN half is IN-FLIGHT on this branch:**
  - **Scalars Phase 0 (EXPAND) — done.** `integer?` / `text?` / `S?` parse in every type
    position (decl, param, return, `as`-cast, nested). Today a no-op (plain types are still
    nullable). Regression: `tests/scripts/25-scalar-optional-syntax.loft`.
  - **DN4 (narrowing-cast enforcement) — done, default-ON** (opt-out `LOFT_NO_DN4`,
    `operators.rs:1674`). A narrowing integer cast of a not-provably-fit value is a compile
    error; `as τ?` is the checked form. DN4 is integer-range-only, needs none of the scalar
    `τ?` representation, so it **shipped ahead of the scalar default flip**. The error
    baseline caught a real silent overflow (`big as i32` was `10000000000`). Regression:
    `tests/dn4_cast.rs` (3) + the `tests/scripts/389-narrow-*` family.
  - **N-Arith range-tracking — done.** `&` and `%` narrow the static range so masked/modded
    values are provably-fit (feeds DN4's fit proof). Regression: `389-narrow-alias-ranges`,
    `389-narrow-vector-full-range`.
  - **Scalar `τ?` representation — BUILT (Step 1, `d121f94c`).** `Type::Optional(Box<Type>)`
    is in `src/data.rs` with the idempotent `Type::optional` former (N-Idem + normalises
    `Optional(Never|Null)`) and `peel_optional`/`base`. 8 exhaustive `match Type` sites
    handled (peel for the layout-agnostic majority; `τ?` rendering in `name()`). Compile-time
    only, sentinel storage, nothing constructs it yet → additive, suite unchanged. N-Idem
    pinned by a unit test. Design: [scalar-optional-representation.md](scalar-optional-representation.md).
- **gridmesh 0.1.2 published (2026-06-29).** The DN4 cutover masked the gridmesh + hex_world
  fixtures and relocked `audience_crystal` to gridmesh 0.1.2; the suite was RED until that
  version was signed into the registry index. It is now published + signed (registry commit
  `056c08c`), which is why the suite is green.

## The one invariant (what the whole rewrite installs)

> `vector<τ>` is dense and uniform for every `τ` (incl. generic `N`); nullability is
> carried only by an explicit `τ?`; lookup-partiality only by the fallible ops
> (`v[i] ⇒ τ?`, etc.). No implicit container rewrite, no implicit unwrap. (The integer
> model applied to null — one former, representation derived.)

---

## Landed ledger (what is true on this branch, both backends)

| Area | Phase | State |
|---|---|---|
| Vectors `vector<S?>` | 0–2 EXPAND/MIGRATE/CONTRACT | ✅ on `main` (`#467`) |
| Borrow elision Tier-0 + Tier-1.5 | — | ✅ on `main`, default-on (`#468`) |
| #465 borrowed-view over-free | 4 | ✅ on `main` |
| Scalars `integer?`/`S?` syntax | 0 EXPAND | ✅ this branch (no-op) |
| DN4 `as τ` fit-check / `as τ?` | 3 TIGHTEN (early) | ✅ this branch, default-on |
| N-Arith `&`/`%` range-tracking | 3 support | ✅ this branch |
| Scalar `τ?` = `Type::Optional` | 0/2 prereq | ✅ Step 1 built (`d121f94c`, `tuxedo-pln85`) |
| Borrow elision Tier 1 (local source) | — | 🔵 implemented, **parked off** by design |

---

## NEXT STEPS — concrete, in order, each with its validation gate

The remaining critical path to "done" (full null-model coherence) is the **scalars half**,
then **Phase 3 DN3/DN2**, then **Phase 5 cleanup**. Each step ends green; never carry two
phases' breakage at once.

### Step 1 — Build `Type::Optional(Box<Type>)` — ✅ DONE (`d121f94c`, `tuxedo-pln85`)
The variant + idempotent `Type::optional` former (N-Idem; normalises `Optional(Never|Null)`)
+ `peel_optional`/`base` are in `src/data.rs`. 8 flagged `match Type` sites handled: the
layout-agnostic majority peel to the base (Optional shares the base's sentinel runtime
layout), `name()`/`short_type` render `τ?`, `for_each_child` visits the child. Compile-time
only, additive — nothing constructs `Optional` yet, so the suite is unchanged (0 failures,
both backends); N-Idem pinned by a unit test. `IntegerSpec.not_null` reconciliation deferred
to DN1/DN3 (per `scalar-optional-representation.md`), as designed.

### Step 2 — Scalars Phase 1 MIGRATE: annotate nullable scalar/field sites with `?` — IN PROGRESS
While the scalar default is STILL nullable, mark every site that genuinely holds null.
Under today's default these are **no-ops** — pre-position them before the flip can surprise them.

**Survey done (2026-06-30) — the blast radius is SMALL.** Raw null-signal counts
(`= null` / `?? ` / `==/!=null`): **default/ (stdlib) = 0** · **lib/ ≈ 20** · **tests/ ≈ 867**.
But the raw counts are dominated by sites that are **NOT scalar/field migration targets**:
- **vector-/lookup-coalescing** (`v[i] ?? d`, `obj.field_lookup ?? d`) — already correct
  (the vectors half made `v[i] ⇒ τ?`); the `??` discharges it. (~all of audience_crystal's.)
- **inferred locals** (`nr = def_names[name].nr`, `l = data[index]`, `x = null`) — nullability
  is *inferred* from the fallible source, not a declared scalar type; inference-governed.
- **`==/!=null` on references/enums** — heap-nullable already (separate from the scalar flip).

The **genuine MIGRATE targets** are *explicitly-typed scalar fields/vars that hold null*. In
the **controlled surface (stdlib + lib) there is exactly ONE**: `Code.cur_def: i32` in
`lib/code.loft` (`self.cur_def = null` at `end_define`) → annotated **`cur_def: i32?`**
(`i32?`/`text?` verified to parse as a no-op both backends). The stdlib needs none.

So pre-annotation is light; the codebase carries null via fallible-lookup (handled) far more
than via nullable scalar fields. The remaining test-side sites are mostly intentional null
tests / inferred locals — left to **Step 3's flip**, which surfaces any genuine miss as a
loud error fixed one-character (`?`), exactly the design's catch-all. **DN1 blast-radius
estimate: very low for the shared surface.**
- **Validation:** suite stays green after annotation (no behaviour change) — ✅ `find_problems`
  0 failures both backends after the `cur_def` annotation.

### Step 3 — Scalars Phase 2 CONTRACT: flip the scalar/field default to non-null (DN1)
The default flip. `IntegerSpec.not_null` default `false → true` (and the bool/char/text
analog); the plain-type parse stops meaning nullable; `not null` becomes an **accepted no-op**.
- **Where:** `src/data.rs` (`not_null` defaults at the `IntegerSpec` constructors, lines
  ~93–167) + `src/parser/definitions.rs` (the scalar `not null` parse — consume + set nothing).

> **⚠️ SCOPING (2026-06-30, before flipping) — DN1 is bigger than "flip + one-char sweep",
> and is intertwined with DN3.** Read this before starting:
> 1. **The flip alone produces WARNINGS, not the clean errors implied.** The type-checker's
>    only consumer of a type's nullability is `expr_not_null` → the **redundant-null-check
>    *warning*** (`operators.rs`, @PLN46 W2). Flipping the default fires that warning on
>    *every* `int == null`/`int != null` (now "always-redundant") — noise, not the bounded
>    error-sweep. **The hard rejection of `x: integer = null` is DN3's `(N-Store)` check,
>    which does not exist yet.** So DN1's flip is only *meaningful + cleanly-bounded* once
>    `(N-Store)` lands — **DN1 and DN3 are one step, not two.**
> 2. **Non-Integer scalars (`text`/`bool`/`char`/`float`) have NO `not_null` flag** — their
>    "non-null analog" must be carried by `Type::Optional` (Step 1) + the `(N-Store)` check,
>    i.e. it is the *same* type-checker work as DN3, not a separate flag flip.
> 3. **Prerequisite — wire `τ?` → `Type::optional(τ)`** in the scalar parse (today Phase-0
>    `?` is a no-op). This is additive, BUT constructing `Optional` exercises **non-exhaustive
>    `match Type` sites with a `_` arm** (Step 1 fixed only the 8 *exhaustive* ones) — an
>    `Integer`-special match with a `_` fallthrough would mis-handle `Optional(Integer)`
>    *silently*. That audit (find the `_`-arm Type matches that must peel) is the real DN1
>    worklist, and it is a correctness audit, not a one-char sweep — **measured surface:
>    ~280 `Type::Integer` match-arm sites across 39 files** (`grep -rn 'Type::Integer' src/`).
> 4. **47 sites read `not_null`**, with double-duty (nullability + bounds).
>
> **Recommended approach:** treat **DN1+DN3 as one gated effort** (an `LOFT_NO_DN1` opt-out
> like DN4, so the suite stays green while sweeping): (a) wire `?`→`Optional`; (b) audit +
> fix the non-exhaustive `_` Type matches to peel; (c) add the `(N-Store)` reject-null check
> gated on; (d) flip the default; (e) `find_problems` sweep the `.loft` misses to `?`, both
> backends; (f) flip the gate default-on. Multi-session; the survey says the *shared-surface*
> `.loft` sweep is small, but the *compiler* work (b)+(c) is the substance.

**Slice (a)+(b) DONE for the current corpus (`8e279c7c`, gated `LOFT_PLN25_OPT` opt-in).**
(a) the postfix `?` constructs `Type::optional` gate-ON (OFF = Phase-0 no-op, byte-identical).
(b) the consuming-site peel audit — **~19 sites peeled** across type-check, layout, interp +
native codegen: `convert` (incl. the null→typed-null transform for a nullable target),
`get_val`/`set_field_check`/`gen_set_first_at_tos`/`generate_var`, `size`×2 +
`element_size`/`element_align` + `typedef` DB-layout (the SIGSEGV — an Optional field got a
wrong record layout, overflowing an adjacent store), `type_def_nr`, `??`
(`handle_null_coalesce`), `null(tp)`, and native `rust_type`/`write_typed_null`/the
`text?`-return ABI. Each behaviour-preserving, a no-op gate-OFF.

**Result: the FULL suite is green gate-ON.** Only **3 `.loft` files** use a `?` annotation
today (25-scalar-optional-syntax, 81-iterator-protocol, + the lib MIGRATE site), so the
*exercised* audit surface is small — all pass on BOTH backends gate-ON; `find_problems`
gate-ON shows no Optional-related failure. The ~280-site count is the THEORETICAL surface
(live only once DN1 makes plain types Optional); for the current `?`-usage the sweep is
complete. **The gate stays opt-in** — `?`→Optional default-on is inert until DN1 gives it
teeth, so flipping it early adds risk (unexercised sites) for no value. Validation: gate-OFF
byte-identical; fmt + both clippy clean.

**NEXT = slice (c)–(f) = the DN1+DN3 effort:** add the `(N-Store)` reject-null check (gated),
flip the scalar default non-null, sweep the `.loft` misses, then flip the gate default-on.
This is where the ~280 sites become *live* (plain types → Optional) — the big multi-session
phase the scoping above describes.

**Slice (c) `(N-Store)` — SCOPED (gate `LOFT_PLN25_DN3` added, implies OPT; check reverted).**
First attempt put the reject-un-discharged-`τ?` check in `convert()` — **wrong granularity.**
`convert` also services COMPARISONS, so it wrongly flagged `s.a == null` (the null-CHECK that
is how you *test* nullability) as an illegal nullable→non-null use. Reverted. **Finding:
`(N-Store)` must live at the STORE / decl / index / return sites (the design's per-site
`N-Store`/`N-Decl`/`N-Coal`/`N-Match` checks), and `== null` / `!= null` null-compares must stay
legal on a nullable.** The probe confirmed the *enforcement direction* is right: an
un-discharged `bad: integer = e.hp` errors; `e.hp ?? 0` passes; and it surfaced a genuine
sweep target — `lib/code.loft`'s `definitions[cur_def]` uses the annotated `cur_def: i32?` as a
non-null index (29 sites) → must discharge post-DN1.

**Store-site implementation DONE (`def34450`):** a `n_store_violation` helper called at the
STORE sites — the typed scalar assignment (`expressions.rs`) + field construction
(`objects.rs`). Right granularity confirmed: the 25-probe (`s.a == null`) is GREEN DN3-ON on
BOTH backends (the convert false-positive is gone), `bad: integer = e.hp` errors, `?? 0`
passes. Gated `LOFT_PLN25_DN3`.

**INDEX site DONE (`65ef931e`):** a nullable cannot be a vector index (`fields.rs:783`,
`n_store_violation(&index_t, &I32, "a vector index")`).

**RETURN site DONE (this commit):** all three return store-paths now run `(N-Store)` — the
explicit `return e` (`control.rs::parse_return`), the implicit function-tail `{ … e }`
(`control.rs::block_result`, gated to `context == "return from block"` so an `if`/`match` arm
whose `result` is legitimately nullable is untouched), and `lhs ?? return e`
(`operators.rs::build_null_coalesce_return`). Matrix probed on BOTH backends: an un-discharged
`integer?` returned into a non-null return errors gate-ON (single diagnostic — `convert` still
peels `Optional`, no double-diagnose); `?? d` / a nullable return type pass; gate-OFF
byte-identical (the value still flows through as the null sentinel). Artifacts:
`bytecode-comparisons/25-nstore-return-{LEGAL,VIOLATION}.loft`.

All store sites for the current corpus are now covered.

**DN1 `_`-arm AUDIT — COMPLETE (`dn1-audit/findings.md`).** Before the default flip, the audit
that the scoping below calls "the real DN1 worklist" is DONE — 5 parallel subsystem audits +
an empirical instrument (`dn1-audit/optional-flow-instrument.loft`, green both backends). Result:
**69 NEEDS-FIX** sites where an `Optional(τ)` value silently takes a non-Optional `_` arm
(panic / wrong size·align·stride / leak / wrong ABI). They collapse into 7 families with ONE
uniform fix — **peel `.base()` before the type dispatch** (byte-identical gate-OFF, additive):
- **A — layout/size/align** (SIGSEGV/panic, HIGHEST): the sibling-pair misses where slice (b)
  fixed one twin and missed the other — `size`✓/`align`✗ (variables 1753), `type_def_nr`✓/
  `type_elm`✗ (data 4752, the root), `element_align`✓/`tuple_def`-align✗ (data 3971),
  `generation::rust_type`✓/`Data::rust_type`✗ (data 4832, panic). Fix first.
- **B — the `(N-Decl)` gate**: `change_var` (variables 1257) rejects `τ ↔ Optional(τ)` as a type
  change → nullable LOCALS unusable today (`x: integer? = 5` fails — even non-null). This is the
  local-half gate; it makes most interp-codegen sites latent-but-UNREACHABLE until fixed.
- **C** deps/leak holes for `Optional(Text/ref)` · **D** the `text?` return-buffer ABI sub-thread
  · **E** the `matches!`-predicate second sweep (`is_scalar`/`slot_kind`/~40 `Type::Text` ABI
  gates) · **F** feature gaps (`match`/`for`/`+`/`float? == null`) · **G** the empirical bugs
  (E1 native `null` tuple-element → `(())`; E2 `"{x}"` format reject). Full rows + the staged
  fix-sequence in `dn1-audit/findings.md § SYNTHESIS`.

**NEXT = the staged fix-sequence (findings.md): Family A ungated first (`type_elm` neutralises
downstream), then B (`change_var`) → reachable interp peels → C leak holes → E/F → D text? ABI →
G → THEN (d) the default flip (`IntegerSpec.not_null` `false → true`), (e) `.loft` sweep of
misses to `?` (incl. `lib/code.loft`'s `definitions[cur_def]`), (f) flip the gate default-on.**
The bare-`null` var-DECL (`x: integer? = null`) failure is one face of Family B's `change_var`
gate (`x: integer = null` errors gate-OFF too) — settle it there.

### Step 4 — Phase 3 TIGHTEN, the rest: DN2 then DN3 (the measured blast radius, LAST)
DN4 already shipped (above). Remaining, least-to-most breaking:
- **DN2** — remove the implicit `τ? ⤳ τ` unwrap in `convert()` (`parser/mod.rs:1585`). After
  this, `??` / `match` are the only ways down from `τ?`. Breakage: code relying on silent
  unwrap.
- **DN3** — type fit-failing ops as `τ?` (`/`, `%`, `[]`, `parse`, overflow) and make
  `(N-Store)` reject an un-discharged `τ?` into non-null storage. **Biggest blast radius** —
  every `b = a / x` without a `??`. The runtime already nulls (`fill.rs`); DN3 is type-level
  (carry `τ?`) + the discharge check.
- **MANDATORY — measure before DN3 lands:** count sites assigning a fit-failing result into
  non-null storage without `??` (the gating number). Migrate with `?? d` / `as τ?` / a mask.
- **Validation:** green after the discharge migration, both backends. This is the
  "willing-to-rewrite-tests" step — it lands last, after everything else is green, with the
  blast radius counted first.

### Step 5 — Phase 5 CLEANUP: retire `not null`  (ordering is load-bearing)
By now `not null` is a no-op everywhere. Remove it — **in this order**, or the **1015
occurrences across 300 `.loft` files** all become parse errors at once:
1. **Strip `not null` from all `.loft` source** (stdlib, `tests/**`, `lib/` + consumers).
   Mechanical, per-area, re-running the targeted suite after each — behaviour-preserving since
   it is already a no-op.
2. **`grep -rn "not null" --include='*.loft'` is clean** (0 occurrences) — the gate before
   touching the parser.
3. **Remove `not null` from the parser** (`definitions.rs`: scalar ~line 1470, the vector arm's
   `has_keyword("not")`, the keyed arms). After this `not null` is a **syntax error** — the
   retirement is enforced, not conventional.
4. **Docs/skills** — drop `not null` from `LOFT.md`, the `loft-write` skill, `formal/types.md`
   notation, examples. `?` becomes the only nullability marker.
- **Validation:** steps 1–2 keep the suite green throughout; step 3 only "breaks" code step 2
  proved nonexistent. End state: one nullability notation (`?`), no non-null marker.

### Step 6 — Land it: single PR to `main`
Per the `@PLN25` decision, the rewrite reaches `main` via **one final PR** when E2 is fully
coherent (scalars done + Phase 3 + Phase 5). The finishing PR carries `Closes @PLN25`. Before
opening: `git fetch`, confirm the head is a descendant of `origin/main`, full
`find_problems.sh` green on both backends.

---

## Parallel sub-thread (NOT on the critical path) — copy-vs-borrow elision

The performance face of the dense model ([copy-elision-design.md](copy-elision-design.md),
Cluster C / `OWNERSHIP_MODEL.md`). Tier-0 + Tier-1.5 are **default-on, on `main`**.
- **Tier 1 (local-struct source) is IMPLEMENTED but parked off** (`LOFT_ELIDE_T1`,
  `use_analysis.rs:423`). The design's own conclusion (2026-06-27 crawler dogfood):
  **do NOT cut it default-on** — it adds only 2 cold-path borrows, no measured local-sourced
  hot copy exists to capture; keep it as a turn-on-and-compare flag until a consumer surfaces
  one. So this is a *deliberate stop*, not unfinished work.
- **Tiers 2–3** (mutable source; unify assignment + return delivery onto one
  `materialization_mode` predicate — where this meets #465 / Cluster C) are **design-only,
  gated behind a measured need**. Do not build speculatively.

---

## Verify / probe commands

```sh
# @PLN25 SCALAR-HALF GATES (this session, opt-in; cached OnceLock in src/keys.rs):
#   LOFT_PLN25_OPT=1  -> the postfix `?` constructs the real Type::Optional (else a no-op)
#   LOFT_PLN25_DN3=1  -> the (N-Store) teeth (reject un-discharged τ? into non-null); implies OPT
# gate-OFF (both unset) = byte-identical default. Verify the scalar-Optional path:
LOFT_PLN25_DN3=1 loft --interpret tests/scripts/25-scalar-optional-syntax.loft   # green both backends
LOFT_PLN25_DN3=1 loft --native    tests/scripts/25-scalar-optional-syntax.loft
LOFT_PLN25_DN3=1 loft introspect  lib/code.loft | grep -c "vector index"          # 14 (cur_def index sweep targets)
# the (N-Store) catch: a nullable used as a non-null store/index errors; `?? d` discharges:
printf 'fn main(){ v:vector<integer>=[1]; i:integer?=null; x=v[i]; }' > /tmp/n.loft
LOFT_PLN25_DN3=1 loft --interpret /tmp/n.loft   # error: discharge with `?? <default>`

# dense default + nullable opt-in (the core correctness, already green):
loft --interpret /tmp/a.loft               # vector<S?>: v[1]==null -> true
loft introspect /tmp/a.loft | grep main_vector   # vector<S> -> main_vector<S> (dense)
# DN4 enforcement (default-on):
echo 'fn main(){ x = 400 as u8; print("{x}"); }' > /tmp/dn4.loft
loft --interpret /tmp/dn4.loft             # -> compile error: use `as u8?`  (LOFT_NO_DN4 opts out)
# full baseline (expect green: 2564/0):
./scripts/find_problems.sh --bg ; ./scripts/find_problems.sh --wait
```

## Facts a fresh session needs

- **`Type::Optional` is not in `src/data.rs` yet** — Step 1 builds it. The commit
  `885eab1d` only *decided* the representation.
- **The scalar default is still nullable** (`IntegerSpec.not_null` defaults to `false`,
  `data.rs:97–167`). Step 3 flips it. DN4 shipped before this because it is integer-range-only.
- **Family A** (`?? <vector-literal>` codegen panic) is fixed on `main` (`6ea779be`). Do NOT
  re-investigate.
- **#465** (dense borrowed-view over-free) is **fixed** and on `main` — it is no longer a
  blocker; the Tier-3 elision unification is its forward-looking sibling, not a re-fix.
- **Branch discipline:** keep `main` off this branch until the scalars half is green at a phase
  boundary; reach `main` via the single final PR (`Closes @PLN25`).
- **Do not build on `2026-07-mac`** — the abandoned enum-synthesis approach, long stale. The
  live `LOFT_E2_SYNTH` references are vestigial from it; the dense-default design (this branch)
  superseded it.
