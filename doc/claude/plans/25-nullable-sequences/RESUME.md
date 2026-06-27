<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN25 dense-default — RESUME HERE (cold-start handoff)

Single resume point for the dense-default value-model rewrite. Written so a fresh
session can pick up after a `/clear`. **Read order:** this file →
[full-design.md](full-design.md) (the consolidated design + the #465 blocker design) →
[storage-vs-access-nullability.md](storage-vs-access-nullability.md) (the invariant +
probe verdicts) → [implementation-steps.md](implementation-steps.md) (the phase order) →
[`../../formal/types.md` § Nullability](../../formal/types.md) (the `N-*` rules).

---

## TL;DR — where we are (updated 2026-06-27)

- **Branch: `pln25-dense`**, off current `main` (`b53e718c`), **pushed**.
- **The flip is LIVE and validated:** `vector<S>` is **dense** (`main_vector<S>`, no
  `__nullable`); `vector<S?>` is the nullable opt-in and `v[1] == null` is **true**.
- **Progress this session — suite 6→2:**
  - **STEP A was already done** — the postfix `?` parses in EVERY type position (decl,
    `as`-cast, return-type, param, nested) via the shared `sub_type_inner` chokepoint.
    The old "decl-only" claim below was against a stale binary. No parser change needed.
  - **STEP B done** (`3d4f99c8`) — the 4 nullable E2 tests annotated to `vector<T?>`
    (json×3 + hash `Counting.entries`×3 defs). All 11 E2 tests green on both backends.
  - **p379 dense regression FIXED** (`73304f37`) — NOT in the old baseline: a vector
    FIELD read off a BORROWED base (for-loop element — `cells = sc.ck_hexes`) was
    deep-copied instead of aliased → write-through lost + null-ref crash (`index … 65535`)
    on both backends. Root: the #415 deep-copy keyed on the syntactic shape; fixed to key
    on the base's `deps` ownership (empty = owns = copy; non-empty = borrows = alias).
    Regression: `tests/scripts/85-store-lifetime-forloop-elem-field-alias-write.loft`.
- **Remaining: 2 failures = #465 / Step C** — both the SAME script
  (`85-store-lifetime-enum-match-borrowed-view-overfree.loft`), once interp
  (`wrap::loft_suite`) + once native (`native::native_scripts`). This is the documented
  enum-match borrowed-view over-free (Step C below).
- **Decision (signed off):** dense-default is the approach. The old enum-synthesis line
  on `2026-07-mac` is **abandoned** (superseded, 109 commits stale — do not build on it).
- **Pre-dense baseline binary for working-vs-broken IR diffs:**
  `/home/lima.guest/loft2/target/release/loft` (branch `fix2-crawler`, no dense flip).

## The one invariant (what the whole rewrite installs)

> `vector<τ>` is dense and uniform for every `τ` (incl. generic `N`); nullability is
> carried only by an explicit `τ?`; lookup-partiality only by the fallible ops
> (`v[i] ⇒ τ?`, etc.). No implicit container rewrite, no implicit unwrap. (The integer
> model applied to null — one former, representation derived.)

---

## Failure ledger (run `find_problems.sh --bg`)

The original handoff predicted 7; the real baseline was 6 (json `null_in_the_middle`
already passed; p379 crashed — unpredicted). Now **2** remain after Step B + the p379 fix.

| Test | Class | Status |
|---|---|---|
| `plan25_e2_json::all_null_elements` | nullable-feature | ✅ Step B (`3d4f99c8`) |
| `plan25_e2_json::null_leading_then_present` | nullable-feature | ✅ Step B |
| `plan25_e2_hash::null_in_shared_vector_is_not_indexed_by_the_hash` | nullable-feature | ✅ Step B |
| `plan25_e2_json::null_in_the_middle_…` | nullable-feature | ✅ (annotated `Item?`; passed already) |
| `issues::p379_two_libs_same_struct_name` | dense regression (borrowed-base field copy) | ✅ FIXED (`73304f37`) |
| `wrap::loft_suite` (1 `.loft` script) | **#465 enum-match borrowed-view over-free** | ⬜ **Step C** |
| `native::native_scripts` (same script, native) | **#465 native mirror** | ⬜ **Step C** |

---

## NEXT STEPS — precise, in order

### STEP A — ✅ DONE (was already complete; the gap below was a stale-binary artefact)

Verified: `?` parses in decl / `as`-cast / return-type / param / nested positions on both
backends (the shared `sub_type_inner` chokepoint handles all). The text below is retained
for history only.

The recovered flip wired the postfix `?` ONLY into the declaration path. Confirmed gap:

```sh
# decl WORKS:
echo 'struct S{a:integer} fn main(){ v: vector<S?> = [S{a:1},null]; print("{v[1]==null}"); }' > /tmp/a.loft
loft --interpret /tmp/a.loft        # -> true
# cast FAILS (Expect token):
echo 'struct Item{name:text,value:integer} fn main(){ v="[null]" as vector<Item?>; print("{len(v)}"); }' > /tmp/b.loft
loft --interpret /tmp/b.loft        # -> error: Expect token ;
# return-type FAILS:
echo 'fn f()->vector<S?>{ v:vector<S?>=[]; v } struct S{a:integer} fn main(){print("{len(f())}");}' > /tmp/c.loft
loft --interpret /tmp/c.loft        # -> error: Expect token
```

- The working site: `src/parser/definitions.rs:1710` (`let nullable_elem = self.lexer.has_token("?")`)
  inside `sub_type`'s vector arm.
- **Task:** route the cast (`as` → `parse_type`), return-type, and param type-parse paths
  through the SAME `?`-postfix handling. Find where they diverge from `sub_type` (start at
  `parse_type` `definitions.rs:1424`, `parse_type_full:1546`, `sub_type:1592`, and the `"as"`
  branch in `operators.rs`). Likely the element-type parse inside `vector<…>` is shared but
  the `?` consumption isn't reached on all entries — unify it.
- Additive, **zero-breakage by design** (new syntax nothing-yet-uses). Gate: full suite
  still 7-failing (unchanged), AND the three probes above all pass.

### STEP B — ✅ DONE (`3d4f99c8`) — Phase 1 MIGRATE: annotate the nullable-feature tests

Once `?` parses in casts, annotate the genuinely-nullable sites — turns 7 failing → 3:

- `tests/plan25_e2_json.rs` — the 3 failing tests use `… as vector < Item >`; change to
  `… as vector < Item? >` (the casts at ~lines 89, 103, 117). `all_present_no_null`
  (line ~133) stays dense `vector<Item>` (it has no nulls — already passing).
- `tests/plan25_e2_hash.rs` — `struct Counting { entries: vector<Count>, … }` (lines ~78,
  100, 231) → `entries: vector<Count?>` (the test asserts a null shared-vector element).
- Re-run those four; they go green. Gate: suite 7→3 (the 3 = #465).

### STEP C — Phase 4: fix #465, the dense borrowed-view over-free (the 3 remaining reds)

Design is in [full-design.md § "The remaining blocker — #465"](full-design.md). In short:
a returned **borrowed view** of an element/field (`match c { Filled{items} => items, _ => [] }`,
`return table[idx] ?? d`) is **aliased instead of copied** under dense → over-free. The
borrow→deep-copy decision is **re-derived across 4 delivery sites off the `__nullable`
*shape*** (`vec_match_candidate`, `classify_vector_delivery`, `ref_return`,
`materialize_vector_arms_into`) instead of the carried `deps` ownership fact.

- **Invariant:** a return whose `deps` name a still-live source is COPIED into the retbuf;
  the source is NEVER freed by the copy; an owning return is moved. Read from `deps`, not
  the value's shape.
- **Fix:** consolidate the 4 sites onto ONE `deps`-driven chokepoint (NOT a 5th
  special-case). GitHub issue: **#465**.
- **MANDATORY gate** (this is the corruption-risk path — a wrong move over-frees the
  caller's buffer = UAF the suite won't catch): matrix-first per `loft-codegen` +
  `OWNERSHIP_MODEL.md`. Boundary matrix `{enum-field-view, struct-field-view, whole-arg,
  index-read, local-view} × {match-arm, if-arm, direct-return} × {returned, returned+churn}`,
  asserting **value + length + leak on BOTH backends** + `LOFT_WATCH_STORE`. Index-read
  views stay aliased until the store-reuse substrate is proven (the #426B exclusion).
  **Design first, probe each claim, then build.** This is @PLN85 Cluster-A / H10 work.

### STEP D — scalars, then TIGHTEN, then CLEANUP (the rest of the rewrite)

Per [implementation-steps.md](implementation-steps.md):
- **Scalars Phase 0→2:** add `x: integer?` syntax + `(N-Intro)`; survey + annotate nullable
  scalar/field sites; flip the scalar default to non-null (`not null` → no-op).
- **Phase 3 TIGHTEN (last, measured):** `DN2` (drop implicit `τ? ⤳ τ`) → `DN4` (`as τ` fit /
  `as τ?` checked cast — needs a new `OpCastChecked` runtime op) → `DN3` (fallible ops typed
  `τ?`, `N-Store` makes un-discharged null a compile error). Measure the blast radius before
  `DN3`.
- **Phase 5 CLEANUP:** strip `not null` from all `.loft` (no-op by now), THEN from the parser.

---

## Verify / probe commands

```sh
# dense default + nullable opt-in (the core correctness):
loft --interpret /tmp/a.loft               # vector<S?>: v[1]==null -> true
loft introspect /tmp/a.loft | grep main_vector   # vector<S> -> main_vector<S> (dense, no __nullable)
# full baseline (expect 7 failing until Step B/C):
./scripts/find_problems.sh --bg ; ./scripts/find_problems.sh --wait
# the canonical incoherence probe (the "done" target, after Step A+B+C):
#   items = "[ {…}, null, {…} ]" as vector<Item?>;  len 3, items[1]==null true, present kept
```

## Facts a fresh session needs

- **Family A** (the `?? <vector-literal>` codegen panic) is **already fixed on `main`**
  (commit `6ea779be`, regression `tests/scripts/440`) — independent of this rewrite. Do
  NOT re-investigate it.
- **The native conditional-reassign retbuf leak** is fixed on `main` (`603bce54`).
- **#465** is filed (sev:high, store-lifetime); it is Step C here. Workaround on `main`:
  `--native` (the bug is interp-shaped there; under dense it's the over-free).
- **`register_and_lay_out_synth`** (`src/typedef.rs`) is marked `#[allow(dead_code)]` —
  superseded by `nullable_vector_elem`/`copy_unknown_fields`; may revive when Step A adds
  forward-ref `?` positions.
- **Branch discipline:** `main` is the release branch — keep it OFF `pln25-dense` until
  **vectors-green** (Step B + Step C done). Merge only at green phase boundaries. The
  rewrite reaches `main` via a single PR when E2 is fully coherent (the @PLN25 decision).
- **Do not build on `2026-07-mac`** — it is the abandoned enum-synthesis approach, 109
  commits stale.
