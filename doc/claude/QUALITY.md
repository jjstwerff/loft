# QUALITY — Open Issues, Active Designs, Enhancement Plan

This document is the single source of truth for **what's broken, what's
being fixed, and what should be fixed next**.  It replaces the earlier
BITING_PLAN.md (which mixed status, design, and history) and
consolidates the open-issue tracking that previously drifted between
PROBLEMS.md and CAVEATS.md.

Read order:
> **⚠ Reconciled 2026-07-10 — read this before the back half.**  Sections 2–5 below are
> **historical design reference, not a live queue.**  The P54 sprint is complete except one
> residual (the Q1 auto-wrap gap in § Open programmer-biting issues); the B2–B7 compiler
> blockers were AUDITED + CLOSED 2026-05-21 on both backends; C54 (`integer` → i64) LANDED
> 2026-04-21.  § Recommended landing order and § Enhancement tiers § Tier 1 are dated
> **2026-04-13** and name work that has since shipped — do not plan from them.  The live
> queues are [ROADMAP.md](ROADMAP.md) and [STABILITY_ROADMAP.md](STABILITY_ROADMAP.md).

1. § Open programmer-biting issues — the live work queue
2. § Active sprint — P54 — **COMPLETE except the Q1 auto-wrap residual**; kept as design reference
3. § Active design — C54 — **LANDED** 2026-04-21 via @PLAN01; kept as design reference
4. § Compiler blockers — struct-enum bugs (B2…B7) — **all CLOSED 2026-05-21**; kept as design reference
5. § Enhancement tiers — quality investments ranked by leverage (**Tier 1 has shipped**; see the banner)

History and closed items live in [CHANGELOG.md](../../CHANGELOG.md).
Decisions to *not* fix something live in
[DESIGN_DECISIONS.md](DESIGN_DECISIONS.md).

---

## Open programmer-biting issues

| # | Issue | Severity | Status |
|---|-------|----------|--------|
| P54 | `json_items` returns opaque `vector<text>`; `MyStruct.parse(text)` silently zeroes on malformed input | High | **Steps 4 + 5 + 6 + Q1 schema-side COMPLETE 2026-04-14 (single-walker design)**.  Step 4: arena materialiser.  Step 5: `Type.parse(JsonValue)` lowers to one IR call to `n_struct_from_jsonvalue(arg, struct_kt)` regardless of struct shape.  The runtime walker uses `stores.types[struct_kt].parts` to dispatch on each declared field type — primitive (text / integer / float / boolean) extracts with inline Q1 schema-side type-mismatch checks, nested struct recurses on the embedded sub-struct DbRef, JsonValue-typed fields byte-copy verbatim, and `vector<T>` fields iterate the JArray + recurse per element (struct elements call back into the walker).  Step 6: auto-wrap form — text arguments to `Struct.parse(text)` route through `json_parse` internally so legacy code keeps compiling.  **Known gap — CLOSED 2026-08-20 (was: RE-VERIFIED 2026-07-10 on BOTH backends — identical output, a shared semantic defect rather than a parity bug: the auto-wrap path parsed but DROPPED diagnostics).**  The probe below now reads: A reports the parse error, B is unchanged, and C is EMPTY after a successful parse.  See the Q1 row in § Open work.  Probe (`struct Cfg { name: text, port: integer }`, malformed `"{ this is not json"`):

```
A  Cfg.parse(bad)               → json_errors() == ""        (len 0)   ← silent
B  Cfg.parse(json_parse(bad))   → json_errors() == "parse error at line 1 col 3 …"
C  Cfg.parse(good)   [succeeds] → json_errors() == B's error (len 128) ← STALE
   …and c.name == "x", c.port == 7, so the parse really did succeed
```

So malformed input and schema mismatches leave fields null with `json_errors()` **empty** (A), and — worse — after a **successful** parse `json_errors()` still returns the *previous* call's error (C), so a program that checks `json_errors()` to validate a parse **reports failure on correct data**.  The two-stage form `Struct.parse(json_parse(text))` reports both classes correctly.  This is the remaining Q1 shape, and the only genuinely-open row in ROADMAP § S.  All 25 P54 + Q1 acceptance tests green.  Boolean allocator-corruption fix carried forward (`database(elem_size.max(2))` for handle stores).  **All JSON natives ship natively as of 2026-04-14 (commit `7a2329e` cleared `NATIVE_SKIP` and `SCRIPTS_NATIVE_SKIP`)** — `n_json_parse`, `n_json_array`, `n_json_object`, `n_to_json`, `n_to_json_pretty`, `n_kind`, `n_keys`, `n_fields`, `n_has_field`, `n_struct_from_jsonvalue`, etc. all dispatch through `src/native.rs` and run through `cargo nextest run --release --test native` cleanly.  The user-facing typed-impl refactor (making `MyStruct.parse(text)` enforce text-must-be-JSON typing at compile time instead of routing through the runtime auto-wrap) remains an optional follow-up — orthogonal to the JSON correctness work.  `p54_struct_parse_rejects_plain_text` was deleted (tested a rejected design decision). |
| Q1 | `json_errors()` reports byte offset only — no path, no line:column, no context snippet | Medium | **Q1 COMPLETE 2026-04-14** — parser side: RFC 6901 path + line:column + context snippet with caret, all 5 `p54_err_*` acceptance tests green; 8 unit tests in `src/json::tests`; 6 `q1_*` tests for state-clearing.  Schema side: kind checks live inline in the unified `n_struct_from_jsonvalue` walker — primitive fields receiving a wrong JSON variant (and not `JNull`, which signals "absent field" and stays silent) push a `"<Struct>.<field>: expected <KKind>, got <KKind>"` diagnostic to `json_errors()`.  Symmetric across direct fields and `vector<struct>` element fields (same walker code path).  6 `q1_schema_side_*` tests covering type-mismatch, missing-field-silent, clean-parse, vector-element mismatch, text-receiving-number, boolean-receiving-string. |
| Q2 | No free-form object iteration / key listing / quick `kind(v)` peek | Medium | **Q2 COMPLETE 2026-04-14**: `kind` + `has_field` + `keys` + `fields` all shipped with real JObject walks.  `keys` returns field names in insertion order; `fields` returns name + value pairs with full deep-copy (primitives and container values preserved).  See § Q2 below |
| Q3 | No `to_json(v)` serialiser — reads but can't write or round-trip | Medium | **JsonValue side complete 2026-04-14, T.to_json() complete 2026-05-07.**  JsonValue: `to_json` walks all six variants (primitives, empty containers, non-empty containers, nested containers) — full tree serialisation.  `to_json_pretty` adds 2-space indent + one-element-per-line for non-empty containers (empty stay `[]` / `{}`; `"k": v` with single space after colon).  `T.to_json()` / `T.to_json_pretty()` for any user struct ship via the parser-side intercept in `src/parser/fields.rs::field()` lowering to `n_struct_to_json(self_ref, struct_kt)` — which delegates to `Stores::show_json` reusing the existing `ShowDb` schema walker (`json: true` flag flips text → JSON-escaped, field names → quoted, struct-enum variants → `{"VariantName": …}`, JsonValue fields → semantic subtree).  See § Q3 below |
| Q4 | No way to construct `JsonValue` trees in loft code (fixtures, mocking, forwarding) | — | **Q4 COMPLETE 2026-04-14**: all six constructors ship with real behaviour.  `json_null` / `json_bool` / `json_number` / `json_string` wire the primitives directly; `json_array` / `json_object` deep-copy caller-supplied items/fields into a fresh arena via a shared `dbref_to_parsed` + `materialise_primitive_into` helper, handling nested containers.  See § Q4 below |
| P54-U | Two JSON parsers (`src/json.rs::parse` for the new JsonValue path, `src/database/structures.rs::parsing` for legacy `text→struct` direct write) accept different dialects and have different diagnostic surfaces | Medium | **All three phases LANDED.**  Phase 1 (2026-04-14): `Dialect::Strict` / `Dialect::Lenient` enum + `parse_with(input, dialect)` in `src/json.rs`.  Phase 2 (2026-04-14): schema-driven `walk_parsed_into` + `walk_parsed_struct` + `walk_primitive_into` in `src/database/structures.rs`; `Stores::parse` / `parse_message` route through the unified parser; instrumentation confirmed zero success-path fallback hits across the full test suite.  Phase 3 (2026-05-07): legacy hand-rolled scanner already gone (Phase 2 superseded it); `Stores::parse` now ONLY uses the unified `parse_with` + walker path with no fallback (single error format `"line N:M path:X"` via `format_walk_err`).  Plus walker `at`-position threading: `walk_parsed_into` / `_struct` / `_primitive` take a byte-offset hint that struct field calls populate with the field's `key_at`, so leaf type-mismatches now report the field's real position instead of byte 0.  Pinned by `tests/data_structures.rs::p54u_leaf_mismatch_reports_field_position`. |

Items that look open in the historical sections of PROBLEMS.md /
CAVEATS.md but are now closed: P22, P91, P135 / C58, P137, P139, C60,
INC#3, INC#29, P140 (test-harness reordering 2026-04-13), P141 (false
positive — `x#continue` already works), B3 (ref_return Enum arm
2026-04-13 — struct-enum return types now get hidden caller
pre-alloc args just like Reference/Vector), B2-runtime (2026-04-13
— 4-part fix: fill_all enum-field retrofit, parse_constant_value
v_block wrap, fields.rs Sig.Idle wrap, calc.rs sub-struct size=1),
B7 (2026-04-13 — `def_code` null-code path now sets `self.arguments`
and `stack.position` from def attributes for native fns + native
registry aliases `t_9JsonValue_<method>` → `n_<method>` impls),
**C54** (2026-04-21, @PLAN01: `integer` widened to i64 end-to-end;
`long` keyword + `l` suffix removed; 34 duplicate `Op*Long` opcodes
reclaimed), **P184** (2026-04-22, @PLAN02: narrow integer aliases
inside `vector` / `hash` / `sorted` / `index` now honour `size(N)`
via the Option L-minimal `Parts::{Byte, Short, ShortRaw, Int}`
variants), **P185** (2026-04-23, @PLAN05: slot-aliasing SIGSEGV
closed by retiring `place_orphaned_vars` and extending the main
IR walk), **B5** (all layers landed — Layer 1 + 2 on 2026-04-14,
Layer 3 closed as a side-effect of the struct-enum return work
landed in #168→#174; `p54_b5_recursive_struct_enum` is
un-ignored and passing).  See CHANGELOG.md.

---

## Open work — actionable summary

Items below are "what to BUILD" derived from the design content in this document.  Each row links to the section that holds the full design.  Three clusters: JSON, Native runtime, Compiler-blocker.

### The library-CI gate reddens library repos for reasons they cannot cure

Found 2026-08-21 opening the eight `unify-library-ci-fpm` adoption PRs. Six went green; two did
not, **neither for anything in the PR** — both branches only add workflow files. The gate is
versioned centrally (`loft-lang/loft@main`), so a change here reddens every library repo on its
next PR, landing on whoever opens one.

| Repo | Red because | Owner |
|---|---|---|
| `loft-libs-docs` #4 | `markdown/src/markdown.loft:261` — *"Cannot index text with 'unknown'"*. A **forward-referenced** `atx_heading_level` (defined :383, called :252) leaves the slice bound unresolved in pass 1. | ✅ **Already fixed** — `1133e272` + `d822dd91`, unmerged on `tuxedo-pln145`. Goes green when that reaches `main`. |
| `loft-libs-game` #11 | `missing: examples-index.tsv — run 'make examples-index'`. The `exindex` check landed here in `7786d28c` (2026-08-18); that repo's last CI run was 2026-08-17, so its first run after the change is its first red. | ✅ **Cured at the source** — the two cross-repo checks are now ADVISORY in a library repo (see below); this red becomes a job-summary note. Goes green when this branch reaches `main`. |

⚠⚠ **The `exindex` message prescribed a cure the repo does not have — FIXED.** `make
examples-index` maps to a target in **this** repo; `loft-libs-game` has no Makefile at all, and
no library repo carries the generator (CI checks loft out separately as `loft-src`). So the gate
told a library maintainer to run something they cannot run.

The check now tests the **target** repo for the Make target and, when it is absent, names the
form that actually works there:

```
missing: examples-index.tsv (this repo defines worked-example tags)
         generate it from a loft checkout, which owns the generator:
         EXAMPLES_REPO_ROOT=$PWD <loft>/scripts/check_doc_drift.sh write-examples-index
```

⚠ The first cut of that fix tested `-f Makefile` in the **current** directory, which is the loft
checkout the script runs from — so it kept printing the `make` form for every library repo. The
control caught it; without one it would have read as fixed. Coverage is unchanged (the index is
still required wherever tags are defined), which is the half worth keeping.

### ✅ Resolved — the two cross-repo checks are tiered

`examples` and `examples-index` now **gate inside loft and advise in a library repo**, by
this repo's own rule that a diagnostic gates iff ignoring it can produce a wrong result. A
dangling doc citation is a broken link, so it advises — loudly: the findings go to the
library PR's job summary in full, with the runnable regenerate command, which is the
`compat check` pattern (*"ADVISORY, never gating"*) applied to docs.

⚠ The scanner's own **selftests still gate everywhere** — a scanner that stops following
its documented rules is loft's bug whoever runs it — and `EXAMPLES_GATE=hard` restores
blocking for a repo that wants it. Full rationale: [LIBRARY_DOC_REVIEW.md](LIBRARY_DOC_REVIEW.md)
§ Why a by-hand pass exists.

⚠⚠ **What this does NOT fix, and it is the deeper smell:** `examples-index.tsv` is a
DERIVED file committed into a repo that does not own its generator. Tiering stops it
blocking; it does not stop it going stale, and the honest cure is to generate it in CI
rather than commit it. Left open deliberately.

⚠ The `loft-libs-docs` row is the sharper lesson: **a published library stopped compiling and
nothing said so for three weeks**, because that repo's CI had not run since 2026-07-28. A gate
that only runs on a PR cannot report a break that arrives from outside the repo.

### JSON cluster

| Item | Section | Status |
|---|---|---|
| **P54** — `JsonValue` enum (active sprint) | [§ Active sprint — P54](#active-sprint--p54-jsonvalue-enum) | Multi-step transition from text-based JSON to first-class `JsonValue` enum |
| **Q1** — JSON parse-error diagnostics | [§ Active design — Q1](#active-design--q1-json-parse-error-diagnostics) | **CLOSED 2026-08-20.**  The one-stage `Struct.parse(text)` filed its diagnostics only under `#errors`, and `json_errors()` reads the other register — so the documented JSON pairing answered `""` for malformed input AND for a schema mismatch, while the program read a struct of zeros.  Nothing cleared the json register there either, so `json_errors()` after a SUCCESSFUL one-stage parse still returned an earlier call's error (a validator reporting failure on correct data), and a `vector<T>.parse` reported an error naming a different type entirely.  One entry point now files both: cleared on entry, written on failure.  Guarded by `tests/scripts/json-one-stage-parse-reports.loft` on both backends |
| **Q2** — free-form object iteration + kind peek | [§ Active design — Q2](#active-design--q2-free-form-object-iteration--kind-peek) | API for iterating untyped JSON + peeking at value kinds |
| **Q3** — `to_json` serialiser | [§ Active design — Q3](#active-design--q3-to_json-serialiser--struct-serialisation) | Symmetric serialisation API to `Type.parse()` |
| **Q4** — `JsonValue` construction in loft code | [§ Active design — Q4](#active-design--q4-jsonvalue-construction-in-loft-code) | Builder API for constructing `JsonValue` trees in loft |
| **P54-U** — unified JSON parser | [§ Active design — P54-U](#active-design--p54-u-unified-json-parser) | Phase 3 deletes ~540 lines of legacy scanner |

**Q3 correctness — narrow nullable fields serialised wrong, fixed 2026-08-20.**
`T.to_json()` delegates to the `ShowDb` schema walker, and that walker re-derived "is this
slot null?" per width instead of asking `Stores::is_null`. Every re-derivation compared a
DECODED value against a RAW sentinel, so on one struct it produced three wrong values in a
single line:

```
struct Cfg { name: text, port: u16?, gain: i16?, level: integer limit(10,255)? }
Cfg { name: "x", port: null, gain: -300, level: null }.to_json()
  was  {"name":"x","port":-2147483648,"gain":-301,"level":265}
  now  {"name":"x","gain":-300}
```

`port` was an absent value serialised as a number, `level` likewise, and `gain` was a
PRESENT value one too low — a nullable SIGNED narrow slot sacrifices its bottom value to
the null code, so its ops encode against `min + 1` while the schema carried the declared
`min`. All three are on the wire, so a consumer of that JSON read numbers where the value
was absent. Fixed at the one home (`is_null` + `IntegerSpec::part_min`); guarded by
`tests/scripts/narrow-null-render.loft` on both backends.

**Q1/P54 — an ABSENT nullable field does not read back null at every width (OPEN, S).**
The fix above stopped `to_json` from masking this, and it is worth stating precisely
because it is the round-trip's remaining asymmetry. Parsing a JSON object that omits a
nullable field — or gives it an explicit `null` — lands:

| field type | absent field reads back |
|---|---|
| `u8?`, `integer limit(lo,hi)?`, `integer?`, `float?`, `text?` | `null` ✅ |
| `u16?` | **`0`** |
| `i16?` | **`-32767`** (the slot's usable minimum) |
| `boolean?` | **`false`** |

Identical on `--interpret` and `--native`, and identical on `main` — this is pre-existing,
not a regression of the render fix. The suspect is the field-DEFAULT writer
(`structures.rs`'s clear-to-default arm), which writes the 2-byte null as the *value*
`65535` where `Parts::Short`'s encoding reserves the raw code `0`; the boolean arm writes
`false` where @PLN17's tri-state reserves `255`. It is the same class as the render bug —
a sentinel written by one site and read by another, with the two encodings disagreeing —
so it belongs to Cluster D and should be fixed at `Stores::is_null`'s write-side twin, not
per width. `silent-wrong`: a null becomes a plausible number with nothing reported.

**`record#errors` is a text, and the docs said "iterate" — OPEN (design, XS-if-decided).**
Both `STDLIB.md` and `LOFT.md` showed `for e in user#errors { log_warn(e); }`.  That
iterates NOTHING: the accessor returns one newline-separated text, the for-loop
re-evaluates it per iteration, and the read CLEARS it — so the first evaluation empties the
register and the loop ends.  Bound to a variable first it iterates the message's
CHARACTERS, which is what `for` over a text means.

Measured, not inferred: `for e in a#errors` → 0 iterations; `t = a#errors; for e in t` → 14
(the message's length).

The docs now show the shape that works (read it into a variable and test it), so nobody is
taught a no-op.  What is NOT decided is whether the accessor should BE a collection —
`vector<text>`, one entry per error — which is what the old text promised.  That is a
compatibility call, not a bug fix: `tests/scripts/197-struct-json-parse-errors-dest.loft`
depends on the text shape (`len(b1#errors) > 0`, `b2#errors != empty`,
`"[{b3#errors}]"`) and on the clear-on-read, and it says so in its own comment.  Deciding it
belongs to [COMPATIBILITY.md](COMPATIBILITY.md)'s process, not to a fix pass.

### Native runtime cluster

| Item | Section | Status |
|---|---|---|
| **Dep-inference** — for native fn returns (zero-leak unblock) | [§ Active design — Dep-inference](#active-design--dep-inference-for-native-fn-returns-zero-leak-unblock) | **SHIPPED; row was stale — re-measured 2026-08-20.**  The inference is in `parser/definitions.rs` (a native `;`-terminated fn with a `self` parameter returning the SAME struct-enum gets `dep=[0]`), and the bite it was written for is gone: 1200+ constructor calls and 1000 accessor-chain calls in loops leave the ledger balanced (1213 allocs / 1211 frees, peak 9 live) on both backends.  Guarded by `tests/scripts/json-value-chain-ownership.loft`.  See the caveat under § Active design — Dep-inference: the inference no longer changes any behaviour I could measure |

### Compiler-blocker cluster

| Item | Section | Status |
|---|---|---|
| **B2-B7** — struct-enum bugs gating P54 | [§ Compiler blockers](#compiler-blockers--struct-enum-bugs) | **AUDITED + CLOSED 2026-05-21 on BOTH backends.**  18 `p54_b*`/`b7_*` interpreter guards green (0 ignored); the one native-only residual (@P301 — struct-enum returned via an intermediate local) was fixed the same day (added the `Type::Enum(_, true, _)` arm to `add_defaults`) and is guarded cross-mode by `tests/scripts/121-struct-enum-return-local.loft`. |

### Store-lifetime cluster

| Item | Section | Status |
|---|---|---|
| **Cluster III Route 2** — reassignment store-free across shared blocks | [plan-57](plans/2-vector-store-watermark/cluster-III-reassignment-pin.md) | **CLOSED 2026-08-21 — shipped, default ON.**  A local reassigned across sibling `if`/`else if`/`match` arms kept EVERY arm's store to scope exit, so the watermark grew with the number of reassignment SITES and not with how many ran: a 16-site function measured peak 20 whichever single arm was taken.  Route 2 (`recover_backer`) confines each block's store to its block — peak is now a flat 5 at 2/4/8/16 sites.  It had sat behind `LOFT_CONF_RECOVER` since 2026-06 pending an un-gate decision; re-measured on 4232 tests (2.2× the evidence it was parked on) and un-gated.  `LOFT_NO_CONF_RECOVER=1` is the opt-out and the first bisect step for a wrong answer in such a function.  Soundness is `store_dead_after_block`, not the flag: a local READ after the blocks does not confine, because freeing a confined store while the local still holds it returns the wrong element on the branch NOT taken.  Both branches verified green (4232 each), values identical on both backends, pinned by `tests/scripts/reassign-across-sibling-blocks.loft`. |
| **@PLN85 cluster I** — FFI struct-return read gap (latent) | [@PLN85 README probe 01](plans/85-store-lifetime-retirement/README.md) | **Latent / unreachable** residual of the now-closed @PLN85.  A `#native` fn returning a non-vector **struct** has no `alloc_struct` helper to lay out a loft-readable ref, so the read path can't be exercised — gated on a future struct-return helper.  The reachable FFI **vector**-return instances (#409/#410) are FIXED.  Re-probe when the helper lands. |
| **@PLN85 cluster IV** — @PLAN51 hidden-buffer-aliasing latent residuals | [@PLN85 README cluster IV](plans/85-store-lifetime-retirement/README.md).  Cites the `@PLAN51` probe set, preserved at [`plans/finished/51-hidden-buffer-aliasing/`](plans/finished/51-hidden-buffer-aliasing/) — the **legacy local plan** `@PLAN51`, *not* the tracker issue `@PLN51` (which is "[audience] Bumper-airplanes") | **RE-MEASURED 2026-08-21: 3 of 62, not ~11, and INTERPRETER-only — the row's "native-mostly" was inverted.**  All 62 probes run clean on both backends (exit 0); `40-nested-tuple-of-canvases`, `47-tuple-local-not-return` and `51-tuple-as-arg` leak on `--interpret` and are clean under `LOFT_NATIVE_LEAK_CHECK=1`, matching @PLAN51 cluster II's own note that *"`--interpret` leaked; `--native` was always clean"*.  The lambda/operator-vector-return, capture-heap-return and mixed-lit-call shapes the row named are now clean.  All three reduce to ONE mechanism, filed as **loft#1051** and RE-MEASURED 2026-08-21: destructuring a tuple bound to a NAMED LOCAL does not leak one record — the **live store set grows LINEARLY with the loop** (peak 24/84/324 at 10/40/160 iterations, against a flat 5 for the same loop reading `t.0`), and the free count is identical in both, so the copies are never released during the loop at all.  Root cause confirmed by `LOFT_VAR_TABLE`: the binding reads `deps=[t]` while `materialize_tuple_element` has given it its OWN store via `OpDatabase` + `OpCopyRecord`, so `owns_store` answers false and no free is emitted — the dependency is a lie about the value.  Attempted and reverted: stripping that dep on the copy path changes nothing, because the chain is `ca -> t -> b` and the tuple is itself modelled as borrowing an element.  Who owns a tuple's records is the ownership-model question that has to be answered first; freeing the binding while some path genuinely makes it a view would trade this for a use-after-free.  Raised to `sev:high`.  **Instrument note:** `LOFT_STORES=warn` reports NOTHING for these (below its floor, as this row said); `LOFT_STORES=timeline` states leak status explicitly and is what separated 3 from 59. |

| **@PLN130 Q6** — which uncovered copy families should be accepted rather than eliminated | [COPY_DIAGNOSTICS.md § What remains open](COPY_DIAGNOSTICS.md) | **Design question, framing settled.**  The owner's rule is recorded — an accept written at the SITE, never a blanket env exemption, so the decision stays reviewable — but which of the measured families qualify is not chosen.  Blocks nothing: an unaccepted copy is reported, which is the state the model requires. |
| **@PLN130** — the uncovered copy set | [COPY_DIAGNOSTICS.md § What remains open](COPY_DIAGNOSTICS.md) | **RE-MEASURED 2026-08-21 over the WHOLE corpus: 504 distinct uncovered sites across 811 scripts (`--interpret`), not "29 over a 90-script sample"** — `InterpCallReturn` 225, `InterpReassignCall` 177, `InterpRecordBind` 72, `InterpReassignVar` 30; a both-backends sample adds `NativeCallReturn` + `NativeRecordBind`, so **five** origins carry uncovered sites rather than four.  `InterpTupleBind` and `ParserMaterialise` never appear (the latter by design — it is classified `Implicit`).  **The guarantee HOLDS: the `Unknown` bucket is empty, 0 rows corpus-wide**, which is the claim that matters — every emitted copy is attributed to a named emitter.  Head of the set by type: `File` 47, `Box` 41, `A` 24, `S` 23, `__tuple` 13, with the stdlib's own `exists(path)` the most-cited shape (`file(path).format` lifts and copies a whole `File` to read one field — the case `Origin::InterpReassignCall`'s doc already names).  Still a legitimate `Avoidable` resting state under decision 3; what changed is that it is now ranked rather than merely sized.  **Cost is NOT established** — a record copy is cheap beside the syscall, and size should not be read as impact.  L effort. |

For the open programmer-biting issues list (running, not plan-shaped), see [§ Open programmer-biting issues](#open-programmer-biting-issues) above.  For ranked enhancement work, see [§ Enhancement tiers](#enhancement-tiers).  For ordering across all open items, see [§ Recommended landing order](#recommended-landing-order).

---

## Active sprint — P54 (`JsonValue` enum)

**Bite.** `MyStruct.parse(text)` silently returns a zero-valued struct
on malformed JSON — no type check, no runtime diagnostic — contradicting
loft's "static types catch mistakes" promise.

**Decision.** Replace the text-based JSON surface with a first-class
`JsonValue` enum.  `json_parse(text) -> JsonValue` is the one entry
point; `MyStruct.parse(JsonValue)` accepts only the typed tree; the
old `json_items` / `json_nested` / `json_long` / `json_float` /
`json_bool` family is withdrawn.

### Surface (`default/06_json.loft`)

```loft
pub enum JsonValue {
    JNull,
    JBool   { value: boolean },
    JNumber { value: float not null },
    JString { value: text },
    JArray  { items_id: integer },     // arena index — see § B5 workaround
    JObject { fields_id: integer },
}

pub fn json_parse(raw: text) -> JsonValue;
pub fn json_errors() -> text;
pub fn field(self: JsonValue, name: text) -> JsonValue;
pub fn item(self: JsonValue, index: integer) -> JsonValue;
pub fn len(self: JsonValue) -> integer;
pub fn as_text(self: JsonValue) -> text;
pub fn as_number(self: JsonValue) -> float;
pub fn as_long(self: JsonValue) -> integer;
pub fn as_bool(self: JsonValue) -> boolean;
```

Pattern matching falls out of existing struct-enum machinery:

```loft
match json_parse(raw) {
    JObject { fields_id } => { … },
    JArray  { items_id }  => { … },
    JNull                 => println("parse error: {json_errors()}"),
    _                     => println("unexpected root kind"),
}
```

### Status (2026-04-14)

| Layer | State |
|---|---|
| Stdlib enum + surface signatures | **Shipped** (`default/06_json.loft`) |
| Rust JSON parser (`src/json.rs`) | **Shipped** — full RFC 8259, 9 unit tests |
| `n_json_parse` (all variants — primitives + arrays + objects + nested) | **Shipped** — step 4 complete |
| `n_json_errors` | **Shipped** |
| `n_as_text`, `n_as_number`, `n_as_long`, `n_as_bool` | **Shipped** |
| `n_field` (JObject lookup), `n_item` (JArray index), `n_len` | **Shipped** — real arena reads, not stubs |
| `n_kind`, `n_has_field`, `n_to_json`, `n_to_json_pretty` | **Shipped** (Q2 / Q3) |
| `n_json_null`, `n_json_bool`, `n_json_number`, `n_json_string` | **Shipped** (Q4 primitives) |
| `n_keys`, `n_fields` (Q2 vector-returning) | **Shipped** — JObject walk allocates a result vector via `database()` + `vector_append`, deep-copies each name (`n_keys`) or each `JsonField` entry (`n_fields`, including container values via `dbref_to_parsed`) |
| `n_json_array`, `n_json_object` (Q4 containers) | **Shipped** (full deep-copy via `dbref_to_parsed`) |
| `T.parse(JsonValue)` codegen (step 5) | **Pending** |
| `T.to_json()` codegen (Q3 struct serialiser) | **Pending** (mirror of step 5) |
| Acceptance tests | **39+ green, 6 ignored** in `tests/issues.rs::p54_*` |

### Remaining steps

**Step 4 (arena materialisation) — COMPLETE 2026-04-14 (four slices in one day).**

**First slice — empty containers.**  `[]` and `{}` now materialise
as real `JArray` / `JObject` variants rather than the earlier
`JNull`-stub, because they have no children and so don't need the
arena allocator.  Specifically:

- `src/native.rs::n_json_parse` — new branches for
  `Parsed::Array(v) if v.is_empty()` and
  `Parsed::Object(v) if v.is_empty()` that set the correct
  discriminant byte + clear diagnostics.  Non-empty containers
  still fall through to the JNull stub with the "materialisation
  pending" diagnostic.
- `src/native.rs::n_len` — returns 0 for `JV_DISCR_ARRAY` /
  `JV_DISCR_OBJECT` (today every container is empty; when the
  full arena ships this path reads the arena vector length).
- `src/native.rs::json_to_text` (shared by `to_json` + pretty) —
  renders `"[]"` / `"{}"` for container discriminants.
- Regression guards in `tests/issues.rs` (12 total):
  - **Per-surface**: `p54_step4_empty_{array,object}_has_{jarray,jobject}_kind`,
    `…_len_is_zero`, `…_to_json`,
    `p54_step4_empty_array_round_trips_through_to_json`
    (parse→serialise→parse agrees on the empty-array
    discriminant), `p54_step4_nonempty_array_still_stubs_as_jnull`
    (prevents accidental partial impl that would claim wrong
    length).
  - **Cross-integration** (added 2026-04-14): locks the
    interactions between step 4's materialisation and the Q2
    (`has_field`) / Q3 (`to_json_pretty`) / existing (`field` /
    `item`) surfaces so a future refactor can't silently break
    the chain while keeping individual per-surface tests green:
    `p54_step4_empty_object_has_no_field`,
    `p54_step4_empty_{object_field,array_item}_lookup_returns_jnull`,
    `p54_step4_empty_{array,object}_pretty_matches_canonical`.

**Second slice — non-empty primitive arrays.**  Arrays whose
elements are all primitive variants (JNull / JBool / JNumber /
JString — no nested containers) now materialise as real JArray
with elements stored in an arena sub-record inside the root's
store.  The sub-record is allocated via `vector_append` (shared
with the rest of the stdlib's vector plumbing), so the entire
tree lives in one store and frees as one unit.

* `src/native.rs::n_json_parse` — new guarded branch for
  `Parsed::Array(v) if v.iter().all(matches!(Null|Bool|Number|Str))`;
  pre-initialises the JArray items field, calls
  `vector_append` per element, delegates to the helper
  `materialise_primitive_into(stores, slot, child)` for the
  discriminant + payload write.
* `src/native.rs::n_len` — JArray arm now reads the arena
  vector's length word at offset 4 (empty arrays still return
  0 via the `items_rec <= 0` guard).
* `src/native.rs::n_item` — full implementation: dispatches on
  JArray discriminant, walks to the i-th slot via
  `8 + i * sizeof(JsonValue)`, returns a borrowed DbRef into the
  parent's store.  Out-of-range indices / non-JArray receivers
  return a fresh JNull.
* `src/native.rs::json_to_text` — JArray recursive rendering:
  walks each arena slot and recurses via `json_to_text` so
  mixed-primitive arrays serialise correctly.  Empty arrays
  still render `"[]"` via the same branch.
* `materialise_primitive_into` helper — one-line-per-variant
  dispatch that writes discriminant + payload into a
  pre-allocated JsonValue slot.  Shared by the
  vector-append path today; nested-container handler (later
  slice) will call it for leaf rewrites.
* Closed one ignored test: **`p54_parse_array_item_access`**
  (was `#[ignore]` "P54 step 4: parse_array + item() indexed
  access") is now green.  Baseline drops from 8 → 7.
* Regression guards in `tests/issues.rs` (9 new):
  `p54_step4_nonempty_primitive_array_has_jarray_kind`,
  `…_length_correct`, `…_item_0_is_first`,
  `…_item_1_is_middle`, `…_item_out_of_range_returns_jnull`,
  `p54_step4_nonempty_bool_array_item_kind`,
  `p54_step4_nonempty_string_array_item_value`,
  `p54_step4_nonempty_array_to_json_round_trips`,
  `p54_step4_nonempty_array_to_json_text_shape` (e.g.
  `json_parse("[1,2,3]").to_json()` = `"[1,2,3]"`).
* Negative guard retained:
  `p54_step4_nested_array_still_stubs_as_jnull` — arrays
  containing other arrays (`[[1,2],[3,4]]`) still hit the
  stub; the nested-container materialiser is a later slice.

**Third slice — non-empty primitive objects.**  Objects of the
shape `{"k1": v1, "k2": v2, ...}` where every value is a
primitive (no nested containers) now materialise as real
JObject variants with `JsonField { name, value }` entries
stored in an arena sub-record.  Same arena pattern as the
JArray slice, plus a per-element name-text write.

* `src/native.rs::n_json_parse` — new guarded branch for
  `Parsed::Object(v) if v.iter().all(|(_, p)| primitive)`;
  allocates the fields-vector sub-record via `vector_append`,
  writes the name text via `set_str`, then delegates to
  `materialise_primitive_into` for the nested JsonValue slot.
* `src/native.rs::n_len` — JObject arm now reads the arena
  vector's length at offset 4.
* `src/native.rs::n_field` — full implementation: dispatches
  on JObject discriminant, linear-scans the JsonField vector
  comparing each name to the query, returns a borrowed DbRef
  into the matched slot's `value` field (or fresh JNull on miss).
* `src/native.rs::n_has_field` — real implementation: same
  linear scan, returns boolean instead of a DbRef.  No longer
  a forward-compatible stub.
* `src/native.rs::json_to_text` — JObject arm recurses into
  each JsonField slot, writes `"<name>":<value>` pairs with
  the same escape rules as JString keys.
* Closed one ignored test: **`p54_parse_object_field_access`**
  (was `#[ignore]` "P54 step 4: parse_object + field() chained
  access") is now green.  Baseline drops from 7 → 6.
* Regression guards (`tests/issues.rs`, 9 new):
  `p54_step4_nonempty_primitive_object_has_jobject_kind`,
  `…_length_correct`, `p54_step4_nonempty_object_field_{hit,miss}_...`,
  `p54_step4_nonempty_object_has_field_{hit,miss}`,
  `p54_step4_nonempty_object_to_json_text_shape`,
  `p54_step4_nonempty_object_to_json_round_trips`,
  `p54_step4_nonempty_object_mixed_primitive_values`.

**Fourth slice — nested containers.**  Arrays-of-arrays,
objects-of-objects, and arbitrary-depth mixes all materialise
now.  `materialise_primitive_into` (despite its now-anachronistic
name) was extended with `Parsed::Array` and `Parsed::Object`
recursive arms.  Each nested container's items / fields vector
is allocated via `vector_append` in the **slot's own store**, so
the entire tree shares the root JsonValue's store and frees
together (File-pattern arena).  `n_json_parse`'s previous
all-primitive-only guards on the array / object branches were
dropped — both now unconditionally call into the recursive
helper.  The earlier "materialisation pending" stub branch was
deleted (no longer reachable).

* Sites: `src/native.rs::materialise_primitive_into` — added
  Array + Object arms; `src/native.rs::n_json_parse` — removed
  primitive-only `where v.iter().all(...)` clauses, removed
  fallback stub branch.  Simpler control flow.
* Negative-stub regression `p54_step4_nested_array_still_stubs_as_jnull`
  REPLACED with positive `p54_step4_nested_array_materialises`.
* Regression guards (`tests/issues.rs`, 9 new):
  `p54_step4_nested_array_outer_length`, `…_inner_length`,
  `…_inner_item_value` (3-deep navigation),
  `p54_step4_nested_object_chained_field` (chained `.field()`),
  `p54_step4_array_of_objects_field_lookup` (mixed: outer
  array, inner object),
  `p54_step4_object_with_array_field` (mixed: outer object,
  inner array — locks both directions of recursion),
  `p54_step4_nested_array_to_json_text_shape` (`[[1,2],[3,4]]`
  serialises canonically),
  `p54_step4_object_with_array_to_json_text_shape`
  (`{"k":[1,2]}` serialises canonically).

**Step 4 status:** **COMPLETE.**  Every JSON document `json_parse`
now produces a fully materialised JsonValue tree.  The arena
contract holds: one root store per parse, all sub-records frees
together when the root DbRef leaves scope.  Q2 `keys` / `fields`,
Q3 nested-container serialisation, and Q4 container constructors
remain — but they now sit on a working arena, not a stub.
The recursive enum form `JArray { items: vector<JsonValue> }` trips
B5.  Workaround: arena indirection — children are stored in a per-parse
allocation and referenced by integer index (`items_id`, `fields_id`).
The arena is allocated in the **same store** as the root JsonValue so
the entire tree frees as one unit when the root DbRef goes out of
scope (the `File` pattern, not `stores.database()`).

**Current state (2026-04-14 explore walk).**
* `src/native.rs:1316-1323` — `jv_alloc` allocator stub: calls
  `stores.database(words.max(2))` and claims a fresh single-record
  store per JsonValue.  This is the file-pattern bottleneck —
  nested children want to share the root's store, not each get
  their own.
* `src/native.rs:1325-1392` — `n_json_parse` materialises
  primitives (JNull / JBool / JNumber / JString) via discriminant +
  variant-field writes.  Arrays and objects hit the
  "materialisation pending (P54 step 4)" diagnostic at line 1379.
* `src/native.rs:1401-1453` — `n_as_text` / `n_as_number` /
  `n_as_long` / `n_as_bool` extractors are **real** (not stubs).
* `src/native.rs:1461-1488` — `n_field`, `n_item`, `n_len` return
  JNull / `i32::MIN` stubs.

**Step 4 change set.**
1. Extend `jv_alloc` (`src/native.rs:1316-1323`) to accept an
   optional parent store so nested allocations land in the root's
   store.  New signature: `jv_alloc_arena(stores, root, words,
   children_count) -> DbRef`.
2. Extend `n_json_parse` (`src/native.rs:1325-1392`) to walk
   `Parsed::Array` / `Parsed::Object` recursively — materialise
   each child via `jv_alloc_arena(root, …)`, write the arena
   record index as `items_id` / `fields_id` on the variant payload.
3. Replace the three dispatch stubs at `src/native.rs:1461-1488`
   with real implementations: read the discriminant byte, dispatch
   on JArray/JObject, fetch the arena record, index or search by
   name, return `JNull` on absent / OOB.

**Step 5 (`Type::parse(JsonValue)` codegen).**  Per-struct unwrap that
walks the schema, calls `n_field` for each declared field, converts
via the `n_as_*` extractors, stores into the destination.  Site:
`src/parser/objects.rs:568-584` (`parse_type_parse`).  Today that
function is text-only — argument coerced to text, emitted as
`OpCastVectorFromText`.  Step 5 adds a JsonValue-unwrap path branch
before the text branch; step 6 rejects plain text for struct
targets at the same site.

**Field-type matrix** (explicit policy — the P54 bite was silent
field-level zeroing; this spells out the replacement):

| Declared field type | JSON produces | Target value |
|---|---|---|
| `text` | `JString`        | value |
| `text` | anything else    | null text + diagnostic |
| `integer` | `JNumber` (integral) | value |
| `integer` | `JNumber` (fractional) | null + diagnostic (lossy cast) |
| `float` | `JNumber` | value |
| `boolean` | `JBool` | value |
| `T` (nested struct) | `JObject` | recurse `T.parse(subtree)` |
| `vector<T>` | `JArray` | iterate + `T.parse` each element |
| `JsonValue` (explicit typing) | any kind | capture the subtree verbatim — the hybrid case, lets typed ingestion coexist with deferred free-form inspection |
| any | `JNull` | declared default |
| any | missing field | declared default |

**Strict vs. permissive** (opt-in per call):

```loft
u = User.parse(v);                  // permissive (default)
u = User.parse(v, strict: true);    // rejects on any deviation
```

- **Permissive** (default): missing fields, extra fields, and
  type-mismatch leaves keep the declared default.  Every deviation
  appends an entry to `json_errors()` so users can opt in to
  diagnostics even without `strict`.  This matches how loft's
  `null`-sentinel discipline is used elsewhere — absence is not
  failure.
- **Strict**: first deviation returns `null` at the top-level
  `parse` call, and `json_errors()` contains the full list of
  deviations with their paths (via Q1 infrastructure).

**Diagnostic shape** (Q1 path + line:column extend to schema errors):

```
User.parse error at /users/3/age (byte 12847, line 423 col 20):
  expected integer, got JString "thirty"
```

`vector<T>.parse(v)` — when a top-level array maps to a homogeneous
vector of T, the same machinery applies per-element.  Each
mismatched element appends a path `/N` diagnostic.

**Root-shape rules**:
- `T.parse(v)` where `v` is not `JObject` → returns `null`, logs
  `"expected JObject at /, got JArray"`.
- `vector<T>.parse(v)` where `v` is not `JArray` → returns an empty
  vector, logs `"expected JArray at /, got JObject"`.

**Step 6 (gate `MyStruct.parse(text)`).**  Same parser site.  If the
argument type is `Type::Text(_)` and the target is a struct, emit
`"MyStruct.parse expects a JsonValue, got text — call json_parse(text)
first"`.  Migration blocked: `tests/scripts/57-json.loft` and
`tests/docs/24-json.loft` have ~20 legitimate `Struct.parse(text)`
sites that must be rewritten first.

**Step 7 (unignore acceptance tests).**  13 `#[ignore]`'d in
`tests/issues.rs::p54_*`.  Each goes green automatically as the
corresponding layer lands.  Five of those — the text-return-through-fn
family + chained-access — depend on **B7** in § Compiler blockers
below; one fix unblocks all five.

**Step 8 (docs).**  LOFT.md JSON section in pattern-matching chapter;
STDLIB.md JSON chapter; CHANGELOG entry.

### Acceptance

`cargo test --release --test issues p54_` — all 39+ tests green.
Brick Buster / Moros editor read JSON via the new surface.  No call
site in `default/`, `lib/`, or `tests/` uses `Struct.parse(text)`.

---

## ~~C54 — integer i64~~ — LANDED 2026-04-21

Shipped via @PLAN01 (`doc/claude/plans/finished/01-integer-i64/`).
`integer` is i64 end-to-end; `Type::Long` / `long` keyword / `l` literal
suffix removed; 34 duplicate `Op*Long` opcodes reclaimed; binary-format
lint in place; `.loftc` cache removed.  C54.D (Rust-style literal
suffixes) was closed by decision — see `DESIGN_DECISIONS.md`.  User-
facing shape: CHANGELOG.md § "Integer → i64 migration".

---


## Active design — Q1 (JSON parse-error diagnostics)

**Bite.** `json_errors()` today returns `"{msg} (byte {at})"` — a
human-readable message plus the raw byte offset into the source.  For
a 50 KB configuration file or an API response, this is effectively
unusable: users can't tell *which field* failed, what line:column to
open the file at, or what the surrounding JSON looks like.  The whole
P54 pitch is "typed tree catches what `Struct.parse(text)` used to
silently swallow" — that win is half-delivered if the diagnostic on
failure is `byte 12847`.

**Status (2026-04-13).**  Parser side **shipped**.
`src/json.rs::parse` returns `Result<Parsed, ParseError>` carrying
`message`, `byte_offset`, and an RFC 6901 `path`.  Path-stack is
threaded through `parse_object` / `parse_array`.  `format_error`
builds the line:column + context snippet on demand.  `n_json_parse`
calls it; `json_errors()` returns the rich text.

```
err: parse error at line 1 col 9 (byte 8):
  path: /x
  expected digit after `.`
    1 │ {"x": 1.}
      │         ^
```

8 unit tests in `src/json::tests` (path for root / array index /
object field / nested / RFC 6901 escapes; line:col conversion;
format_error covering path / line / col / caret) plus 4
acceptance tests in `tests/issues.rs::q1_*` (path for object
field, path for array index, caret marker present, line+byte
markers present).

**Schema-side still pending**: `Type::parse(JsonValue)` failures
will reuse the same path + format_error infrastructure when P54
step 5 lands.  Recovering parser (continue past first error,
return list of failures) remains a follow-up with its own
trade-offs.

### Target diagnostic

```
parse error at line 423 col 17 (byte 12847):
  path: /users/3/address/zip
  expected digit after `.`
    421 │       {
    422 │         "address": {
    423 │           "zip": 1.}
                          ^
    424 │         }
```

Three pieces, each independently useful:

1. **JSON Pointer path (RFC 6901).**  `/users/3/address/zip` — names
   the field.  Accumulated during descent: push `/users` entering
   that object's field, push `/3` entering the array element, …  On
   error, the current path is the location.  Storage: `Vec<String>`
   in the parser; push on descent, pop on ascent.

2. **Line:column.**  One pass over `bytes[0..offset]` counting `\n`
   converts the byte offset at error time.  O(n) but only executed
   on failure, not per token.

3. **Context snippet.**  Two lines before, the error line with a
   caret under the offending byte, one line after.  Trivial once
   line:column is known.

### Surface changes

**`src/json.rs`:**

```rust
pub struct ParseError {
    pub message: String,
    pub byte_offset: usize,
    pub path: String,        // RFC 6901 pointer; "" for root
}

pub fn parse(input: &str) -> Result<Parsed, ParseError>;
```

Internal parser functions gain a `&mut Vec<String>` path stack.
`parse_object` pushes `/escape(name)` before recursing on each
field's value, pops after.  `parse_array` pushes `/{index}` and
pops the same way.  Push/pop is O(1); no extra allocation per
token.

RFC 6901 escaping: `~` → `~0`, `/` → `~1`.  Five-line helper.

**`src/native.rs::n_json_parse`:**

```rust
Err(ParseError { message, byte_offset, path }) => {
    let (line, col) = line_col_of(raw.as_bytes(), byte_offset);
    let snippet = context_snippet(raw, byte_offset, 2, 1);  // 2 before, 1 after
    stores.last_json_errors.clear();
    stores.last_json_errors.push(format!(
        "parse error at line {line} col {col} (byte {byte_offset}):\n\
         \x20 path: {path}\n\
         \x20 {message}\n\
         {snippet}"
    ));
}
```

Multiple errors: keep `Vec<String>` shape; future step (not this
landing) can teach the parser to continue past recoverable errors
— `json_errors()` would then return one line per failure.  For
today's single-error-at-first-fail parser, the Vec holds one well-
formatted entry.

**`default/06_json.loft`:**
No change — `json_errors()` signature (`-> text`) is already the
right shape.  What callers *see* in that text becomes useful.

### Implementation cost

~60 lines in `src/json.rs` (`ParseError` struct, path-stack plumbing
in 6 parse functions, RFC 6901 escape helper, line:column converter,
context-window formatter).  ~20 lines in `n_json_parse` to replace
the tuple-destructure with the rich format.

### Tests (landed 2026-04-14)

All five spec-named acceptance tests live in `tests/issues.rs`:

- `p54_err_reports_path_into_nested_object` — parse of
  `{"a": {"b": 1.}}` reports `/a/b`. ✅
- `p54_err_reports_path_into_array_element` — parse of
  `[1, 2, 1.]` reports `/2`. ✅
- `p54_err_reports_line_and_column` — 3-line input fails on
  line 2, diagnostic contains `line 2`. ✅
- `p54_err_context_snippet_includes_caret` — snippet carries
  a `^` under the offending column. ✅
- `p54_err_path_escapes_slash_and_tilde` — a field named
  `a/b~c` renders as `/a~1b~0c` in the diagnostic (RFC 6901
  escape round-trips through `n_json_parse`). ✅

Supporting coverage:
- `src/json::tests` — 8 unit tests covering `parse` path
  threading (root / array / object / nested / RFC 6901),
  `line_col_of` on simple + multi-line input, and
  `format_error` shape (path / line / col / caret / message).
- `tests/issues.rs::q1_*` — 6 acceptance tests covering
  state-clearing (`cleared_after_successful_parse`,
  `empty_after_clean_parse`), path substrings, and format
  shape assertions.

### Why Tier 2 (not Tier 1)

This doesn't unblock any ignored test and doesn't close a crash.
It's an *ergonomics* win that substantially improves the P54 value
proposition.  Landing it inside the P54 sprint — between step 5
(`Type::parse(JsonValue)`) and step 6 (`.parse(text)` rejection
diagnostic) — is natural: step 6 will want to print a useful
diagnostic when users pass text, and that diagnostic can reuse the
line:column + context-snippet helper.

### Schema-side reuse (P54 step 5)

`Type::parse(JsonValue)` generates its own deviations (missing
required field, type mismatch at a leaf, wrong root kind).  These
reuse the same path + line:column + snippet infrastructure:

```
User.parse error at /address/zip (byte 2047, line 48 col 20):
  expected integer, got JString "10012"
```

Implementation: schema codegen passes its current path (struct
field name or `/N` for vector elements) into the same formatter
used by the parser.  No second diagnostic system.

### What this design is not

- Not a JSON Schema validator — the diagnostic reports *where* the
  parser or schema-walker gave up, not *what a user's business
  rules* expected.
- Not a recovering parser — first parser error still stops.  A
  recovering mode is a follow-up with its own design trade-offs.

---

## Active design — Q2 (free-form object iteration + kind peek)

**Bite.** A user holding a `JsonValue` of unknown shape has no way
to list an object's keys or iterate its fields.  `JObject {
fields_id }` exposes an arena index, not something loopable.
Without this, "free-form" reduces to "guess candidate key names
and try `field()` on each" — which isn't free-form at all.

`match`'s seven-arm dispatch also isn't great for a one-line
"what kind did I get?" peek in logs or conditional branches.

### Surface

```loft
/// Returns the variant name as text: "JNull", "JBool",
/// "JNumber", "JString", "JArray", "JObject".  Cheap — reads the
/// discriminant byte, formats a literal.
pub fn kind(self: JsonValue) -> text;            // ★ LANDED 2026-04-14

/// JObject: returns the vector of declared field names in
/// insertion order.  Any other variant: empty vector.
pub fn keys(self: JsonValue) -> vector<text>;

/// JObject: returns the vector of (name, value) entries so a
/// user can `for entry in fields(v) { … entry.name … entry.value … }`.
/// Any other variant: empty vector.
pub fn fields(self: JsonValue) -> vector<JsonField>;

/// JObject: true if the key is present (even if its value is JNull).
/// Distinguishes "absent" from "present-but-null".
pub fn has_field(self: JsonValue, name: text) -> boolean;
```

`JsonField` already exists in the stdlib for schema-internal use;
this promotes it to the public surface.

### Implementation

- `n_kind` — **LANDED 2026-04-14 in `src/native.rs`**.  Reads the
  discriminant byte at offset 0, returns one of six variant
  names via `stores.scratch` + `Str::new`.  Unknown bytes map
  to `"JUnknown"` defensively.  Registered as both free (`n_kind`)
  and method alias (`t_9JsonValue_kind`).  Guard tests in
  `tests/issues.rs`: `q2_kind_of_jnull_free_form` and
  `q2_kind_of_jnull_method_form` (dispatch), plus one per
  primitive variant (`jbool`, `jnumber`, `jstring`), and
  `q2_kind_of_parsed_primitive` locking the discriminant agreement
  between `n_json_parse` and `n_kind`.

  **B7 note:** this is the first Q2 method that dispatches on a
  `JsonValue` local — shipping it exercised the method-call
  surface that B7 was originally supposed to block.  The method
  form works ok today (`v.kind()`) in both debug and release,
  suggesting that some combination of the B2-runtime retrofit,
  the B5 layer-1/2 fixes, and the `t_9JsonValue_*` method-alias
  registration for the older `n_as_*` / `n_field` / `n_item` /
  `n_len` natives has narrowed B7's actual scope to just the
  character-interpolation text-return path
  (`b7_character_interpolation_return_crashes`, still `#[ignore]`).
  See § Compiler blockers — B7 for the narrowed symptom.

- **`n_keys` — JObject walk LANDED 2026-04-14.**  Returns an
  empty `vector<text>` for non-JObject variants; for JObject,
  walks the fields vector and copies each name into the result
  vector store.  Establishes the vector-from-native pattern for
  text elements: `database(text_size)` claims the handle store
  with the right per-element size; `vector_append` claims the
  inner vector record on first call; `set_str` allocates a
  string sub-record for each name and the new record-nr is
  written into the slot.  Insertion order preserved (linear
  walk).  Registered as both `n_keys` (free) and
  `t_9JsonValue_keys` (method alias).  Regression guards:
  `q2_keys_on_jnull_is_empty`, `…jbool…`,
  `q2_keys_on_jobject_returns_field_names_length`,
  `q2_keys_on_jobject_returns_multiple_field_names_length`,
  `q2_keys_on_jobject_preserves_first_name`,
  `q2_keys_on_jobject_collects_all_names`,
  `q2_keys_for_loop_is_safe`.
- **`n_fields` — JObject walk LANDED 2026-04-14 (full deep-copy
  2026-04-14 PM).**  Mirrors `n_keys`'s walk pattern; each result
  element is a `JsonField` struct.  Names copy verbatim.
  **All value kinds deep-copy** via a shared
  `dbref_to_parsed(stores, src) -> crate::json::Parsed` helper
  that walks the source arena recursively, plus the existing
  `materialise_primitive_into` writer on the result side —
  primitives (JNull / JBool / JNumber / JString) and containers
  (JArray / JObject with arbitrary nesting) all round-trip.
  Regression guards: `q2_fields_on_jnull_is_empty`,
  `q2_fields_on_jstring_is_empty`,
  `q2_fields_on_jobject_returns_field_entries_length`,
  `q2_fields_on_jobject_collects_multiple_entries`,
  `q2_fields_collects_all_names`,
  `q2_fields_preserves_primitive_number_values`,
  `q2_fields_preserves_container_values_array`,
  `q2_fields_preserves_container_values_object`,
  `q2_fields_for_loop_is_safe`.

  **Q2 cross-integration:**
  `q2_full_surface_smoke_on_jobject` exercises kind + has_field
  + keys + fields on the same JObject value and sums to 4 — every
  helper now returns its real JObject answer.

- **`n_has_field` — LANDED 2026-04-14 (stub 2026-04-14 AM,
  real impl 2026-04-14 PM with P54 step 4 third slice).**
  First shipped as a forward-compatible stub returning `false`
  unconditionally (JObject couldn't be constructed at that
  point).  After the step 4 third slice materialised primitive
  JObjects, rewritten to do a real linear scan: dispatches on
  JObject discriminant, walks the JsonField vector, compares
  each name to the query, returns true on first match.
  Primitive variants still return false through the short-
  circuit path.  Registered as both `n_has_field` (free) and
  `t_9JsonValue_has_field` (method alias).  Regression guards:
  - Primitives return false:
    `q2_has_field_on_jnull_is_false`, `…jbool…`, `…jnumber…`,
    `…jstring…`.
  - Dispatch paths:
    `q2_has_field_free_form_on_parsed_primitive`
    (free-dispatch + method-alias lock),
    `q2_has_field_gates_conditional_safely` (control-flow
    pattern).
  - JObject positive + negative (step 4 third slice):
    `p54_step4_nonempty_object_has_field_{hit,miss}`.

### Iteration example

```loft
v = json_parse(raw);
match v {
    JObject { fields_id } => {
        for entry in fields(v) {
            println("{entry.name}: {kind(entry.value)}");
        }
    }
    _ => println("not an object"),
}
```

### Tests (landed)

Coverage shipped under family-prefixed names rather than the
spec names originally proposed; the originals are kept here as
intent labels with a pointer to the actual test set:

- `kind` — `q2_kind_of_jnull_free_form`, `…_jnull_method_form`,
  `…_jbool`, `…_jnumber`, `…_jstring`, `…_parsed_primitive`
  (six assertions across the primitive variants).
- `keys` insertion order — `q2_keys_on_jobject_preserves_first_name`,
  `…_collects_all_names`.
- `fields` iteration — `q2_fields_on_jobject_collects_multiple_entries`,
  `q2_fields_collects_all_names`,
  `q2_fields_preserves_primitive_number_values`,
  `q2_fields_preserves_container_values_array/object`.
- `has_field` absent-vs-null — `q2_has_field_on_jnull/jbool/jnumber/jstring_is_false`,
  `q2_has_field_free_form_on_parsed_primitive`,
  `q2_has_field_gates_conditional_safely`.
- `kind` on intermediate `field()` results —
  `p54_step4_field_on_jstring_returns_jnull` exercises this
  via `v.field("missing").kind()`.
- Cross-surface: `q2_full_surface_smoke_on_jobject` sums to 4.

### Depends on

P54 step 4 (arena materialisation).  Landed immediately after.

---

## Active design — Q3 (`to_json` serialiser + struct serialisation)

**Bite.** The current surface is read-only.  Users who parse a
JSON response, modify a subtree, and want to forward it — or
users building a JSON reply from a loft struct — have no way to
emit JSON text.  Round-trip testing (parse → compare →
serialise → compare) is impossible.

### Surface

```loft
/// Serialise a JsonValue tree to canonical JSON text.
/// Object keys emitted in insertion order; no extraneous
/// whitespace; numbers formatted per RFC 8259.
pub fn to_json(self: JsonValue) -> text;          // ★ primitives LANDED 2026-04-14

/// Pretty-printed variant — 2-space indent, one element per line
/// for arrays/objects with >1 element.  Useful for logs and
/// golden-file tests.
pub fn to_json_pretty(self: JsonValue) -> text;

/// Struct serialisation — inverse of `T.parse(JsonValue)`.
/// Walks the struct's schema, builds a JObject, recurses into
/// nested struct / vector fields.  Fields with null sentinel
/// values serialise as JSON null (or are omitted under
/// `skip_null: true`).
pub fn to_json(self: T) -> text;                  // one per type; codegen-generated
pub fn to_json_pretty(self: T) -> text;
```

**Canonical + pretty — full tree 2026-04-14.**  Both
`to_json(self: JsonValue)` and `to_json_pretty(self: JsonValue)`
ship for all six variants.  Implementation: `src/native.rs`
factors the core rendering into a shared helper
`json_to_text_at(stores, v, pretty, depth)` — `pretty` controls
indent emission, `depth` tracks the recursion level.  Containers
recurse into each child slot; pretty mode emits `\n  …` at depth+1
for each element/field, dedents the closing bracket back to depth.
Empty containers stay `[]` / `{}` (no newline padding either way).
After object keys, pretty inserts a single space after the colon
(`"k": v`).  `n_to_json` and `n_to_json_pretty` are registered as
both free and method-alias forms.

The canonical path dispatches on the discriminant byte, writes
`"null"` / `"true"` / `"false"` for `JNull` / `JBool`, uses
Rust's `f64::Display` shortest-round-trip for `JNumber`, and
applies the canonical escape set (`"` / `\\` / `\n` / `\r` /
`\t` / `\b` / `\f`, plus `\uXXXX` for other control bytes) to
`JString`.  Non-finite numbers serialise as `null` (RFC 8259
constraint).

Regression guards in `tests/issues.rs` (13 total):
- `to_json` (canonical): `q3_to_json_of_jnull`,
  `q3_to_json_of_jbool_true/false`,
  `q3_to_json_of_jnumber_integer/fractional`,
  `q3_to_json_of_nan_becomes_null` (non-finite → `"null"`),
  `q3_to_json_of_jstring_plain` (`"hello"` round-trip).
- `to_json_pretty` (byte-identical to canonical for primitives):
  `q3_to_json_pretty_of_jnull/jbool/jnumber/jstring`,
  `q3_to_json_pretty_free_form` (free-fn dispatch + method-alias
  registration), and `q3_to_json_and_pretty_agree_on_primitive`
  — directly asserts `to_json(v) == to_json_pretty(v)` so a
  future divergence on primitives is caught at the call site.

**Container slice — LANDED 2026-04-14.**  The recursive walk
ships in `json_to_text_at`; the algorithm matches the original
plan (primitive dispatch recursed, escape logic shared between
JString values and JObject keys via a `write_json_string`
helper).  Six new pretty-mode regression guards lock the
indent layout: `q3_to_json_pretty_empty_array`,
`…_empty_object`, `…_array_indents_elements`,
`…_object_indents_fields`, `…_nested_array_in_object`,
`q3_to_json_and_pretty_differ_on_nonempty_container` (asserts
the active divergence so a regression that loses pretty's
indent gets caught).

**Deferred — escape-sequence regressions in `code!()` tests.**
Two additional guards for `"a\"b\\c"` and `"a\nb"` round-trips
were attempted but the first hung the test harness (loft
parser's interpretation of double-escaped strings fed through
Rust's `code!()` macro needs isolated investigation; the
Rust-side escape logic in `n_to_json` is exercised by unit
inspection).  Move escape-sequence repros to standalone
`.loft` files for debugging before re-adding the tests.

### Field-type matrix for struct → JSON

| Field type | Serialisation |
|---|---|
| `text` | `JString` |
| `integer` | `JNumber` (integral) |
| `float` | `JNumber`; `NaN` / `inf` → JSON `null` + diagnostic |
| `boolean` | `JBool` |
| `T` (nested struct) | `JObject` (recurse) |
| `vector<T>` | `JArray` (iterate) |
| `JsonValue` | serialised verbatim (round-trip the captured subtree) |
| null sentinel | `null` by default; configurable |

### Canonical form

- **No whitespace** outside strings (pretty-printed form adds it
  back).
- **Numbers** use shortest round-trip representation (same as
  `{f}` formatter).
- **Strings** escape `"`, `\\`, and control bytes `< 0x20`; UTF-8
  bytes pass through verbatim (no `\uXXXX` escaping of BMP
  characters — RFC 8259 allows both; shortest wins).
- **Object key order** — insertion order for `to_json(JsonValue)`,
  declaration order for `to_json(T)`.  Not sorted — stable
  insertion order is useful for diffing and avoids surprise
  reordering when programs read-modify-write.

### Implementation

- `src/json.rs` gains `pub fn format(v: &Parsed, pretty: bool) ->
  String` — recursive walk writing into a `String` buffer.
- `n_to_json` — reads a `JsonValue` DbRef, walks the arena into a
  `Parsed`-shaped temporary, formats.  Or format directly from
  the arena representation; same cost.
- `T.to_json()` codegen at the struct-method generation site —
  walks the schema, emits `n_build_json_field` calls per field
  into a work-buffer arena, then formats.  Mirror image of step 5.

### Round-trip property

`parse(to_json(v)) == v` for every `JsonValue`.  Property test
asserts this on a generated corpus (null, booleans, numbers
including 0.1-family, unicode strings, nested up to depth 5).

### Tests

- `q3_primitives_round_trip` — each primitive variant.
- `q3_nested_object_round_trip`.
- `q3_array_of_mixed_kinds_round_trip`.
- `q3_pretty_form_valid_json` — `parse(to_json_pretty(v)) == v`.
- `q3_unicode_string_escaping` — `"α β 😊"` round-trips without
  `\uXXXX` escaping.
- `q3_struct_to_json` — `User { name: "Bob", age: 30 }.to_json()`
  produces `{"name":"Bob","age":30}`.
- `q3_struct_with_nested` — recurses into `Address`.
- `q3_struct_with_jsonvalue_field` — raw subtree forwards
  verbatim.
- `q3_null_float_becomes_json_null`.

### Depends on

P54 step 4 for the `JsonValue` serialisation side.  `T.to_json()`
lands after step 5 (same codegen machinery in reverse).

---

## Active design — Q4 (JsonValue construction in loft code)

**Bite.** Today a loft program can read a `JsonValue` but cannot
build one.  Test fixtures ("given this JSON, when I call my
function…"), reply-construction in a web service, and forwarding
synthesised payloads are all impossible.

The obvious syntax — `v = JString { value: "hi" }` — trips
**B2-runtime** (unit-variant / struct-enum literal construction
at runtime crashes).  Waiting for B2-runtime blocks Q4 on
multi-session compiler surgery.

### Surface — helper constructors (bypass B2-runtime)

```loft
pub fn json_null() -> JsonValue;            // ★ LANDED 2026-04-14
pub fn json_bool(v: boolean) -> JsonValue;  // ★ LANDED 2026-04-14
pub fn json_number(v: float) -> JsonValue;  // ★ LANDED 2026-04-14
pub fn json_string(v: text) -> JsonValue;   // ★ LANDED 2026-04-14
pub fn json_array(items: vector<JsonValue>) -> JsonValue;   // blocked on step 4
pub fn json_object(fields: vector<JsonField>) -> JsonValue; // blocked on step 4
```

Plus a struct-literal shortcut for JsonField:

```loft
f = JsonField { name: "age", value: json_number(30.0) };
```

These are **native** functions that allocate arena records
directly — the same path `n_json_parse` uses internally.  They
sidestep B2-runtime because the variant is constructed in Rust,
not via loft's struct-enum literal syntax.

**Primitive slice — 2026-04-14 (four of six shipped).**
`json_null`, `json_bool`, `json_number`, and `json_string` all
landed.  `src/native.rs` grows four `n_json_*` fns, each using
the existing `jv_alloc` helper and the same
discriminant-byte + payload-field layout `n_json_parse` already
writes for parsed primitives.  Registered in `NATIVE_FNS`;
declarations added to `default/06_json.loft` under the
extractors.  `json_number` rejects non-finite inputs (NaN /
±Inf) by storing `JNull` + appending a diagnostic to
`json_errors()`, matching the RFC 8259 constraint.
`json_string` copies the text into the JsonValue's own store so
the returned value lifetime-extends its payload.

Regression guards (`tests/issues.rs`, 9 total):
- `q4_json_null_returns_jnull_variant`
- `q4_two_json_nulls_via_match_works`
- `q4_json_bool_round_trips_true`
- `q4_json_bool_round_trips_false`
- `q4_json_number_round_trips_finite`
- `q4_json_number_negative_finite`
- `q4_json_number_nan_becomes_jnull`
- `q4_json_string_round_trips`
- `q4_json_string_empty`

All guards use pattern-match destructuring for the variant
payload — not method calls — so they ride on the working path
guarded by `b7_multiple_json_parse_via_match_works`, avoiding
the still-open B7 method-surface bug.  The string tests
specifically measure `value.len()` inside the match arm rather
than returning the bound `value: text` (the text-escape path
trips the same native-returned-text lifecycle issue as
`b7_character_interpolation_return_crashes`).

**Container slice (empty input) — 2026-04-14.**
`json_array(items)` / `json_object(fields)` shipped with
empty-input support today.  Implementation: read the input
vector's DbRef from the stack, query its length via
`vector::length_vector`; if 0, build the empty-container
variant via the same path `json_parse("[]")` /
`json_parse("{{}}")` use.  For non-empty input, the
constructors deep-copy each element / field into the new
arena via a shared `dbref_to_parsed(stores, src) -> Parsed`
helper that walks the source JsonValue tree recursively,
and the existing `materialise_primitive_into` writer
materialises each Parsed sub-tree into the destination
root's store.  Nested containers round-trip
(`json_array([json_array([…])])`, objects inside arrays,
arrays inside objects).

* Sites: `src/native.rs::n_json_array`, `n_json_object` —
  each ~30 lines, mirror shape.  Registered as both free
  fns (`n_*`).  Method aliases not added because these are
  free constructors, not methods on a receiver.
* Shared helper: `dbref_to_parsed` (same file) walks a
  JsonValue DbRef tree and produces the transient
  `crate::json::Parsed` snapshot used by the existing
  writer.  Also used by `n_fields` to deep-copy container
  values while walking a JObject.
* Regression guards (`tests/issues.rs`, 13 total):
  `q4_json_array_empty_vector_returns_jarray`,
  `…_empty_has_zero_length`,
  `…_empty_serialises_as_brackets`,
  `q4_json_array_nonempty_input_returns_jarray`,
  `q4_json_array_multi_element_round_trips`,
  `q4_json_array_item_access_after_construction`,
  `q4_json_array_nested_construction`,
  `q4_json_object_empty_vector_returns_jobject`,
  `…_empty_has_zero_length`,
  `…_empty_serialises_as_braces`,
  `q4_json_object_single_field_round_trips`,
  `q4_json_object_multi_field_length`,
  `q4_json_object_serialisation`.

**Container slice (non-empty deep-copy) — LANDED
2026-04-14.**

### Builder ergonomics

For object-heavy construction, a vector-of-fields literal reads
cleanly:

```loft
reply = json_object([
    JsonField { name: "status", value: json_string("ok") },
    JsonField { name: "count",  value: json_number(42.0) },
    JsonField { name: "data",   value: forwarded_subtree },
]);
```

If usage patterns show this is too verbose, a second-round API
(`json_object_of([("status", "ok"), ("count", 42)])` with inferred
variants) can land; deferred until real call sites exist.

### Mutation — deferred

Mutating an existing tree (`v.set_field(name, value)`,
`v.push_item(item)`, `v.remove_field(name)`) is a natural
follow-up but **not in scope** for Q4.  Reason: arena indirection
+ the current `OpFreeRef` discipline make in-place mutation of a
tree's children expensive to reason about.  The construction
helpers above let users build a new tree from parts; replacing a
subtree in a parsed tree can be done by constructing the new
object and handing it to the consumer.

### Tests (landed)

Coverage shipped under family-prefixed names rather than the
spec names originally proposed:

- Primitive constructors — `q4_json_null_returns_jnull_variant`,
  `q4_json_bool_round_trips_true/false`,
  `q4_json_number_round_trips_finite`,
  `q4_json_number_negative_finite`, `…_nan_becomes_jnull`,
  `q4_json_string_round_trips`, `q4_json_string_empty`,
  `q4_two_json_nulls_via_match_works`.
- Array round-trip — `q4_json_array_empty_*`,
  `q4_json_array_nonempty_input_returns_jarray`,
  `q4_json_array_multi_element_round_trips`,
  `q4_json_array_item_access_after_construction`.
- Object round-trip — `q4_json_object_empty_*`,
  `q4_json_object_single_field_round_trips`,
  `q4_json_object_multi_field_length`,
  `q4_json_object_serialisation`.
- Nested construction — `q4_json_array_nested_construction`
  (array of arrays).
- Forward captured subtree — `q4_forward_captured_subtree_array`,
  `…_object`, `…_round_trip` (parse → embed in fresh JObject →
  serialise → re-parse — locks the deep-copy preserves arena-
  origin container values too).
- Pending: `q4_fixture_for_parse` (build tree → hand to
  `User.parse(v)`) — gated on P54 step 5 codegen.

### Depends on

P54 step 4 (arena machinery).  Q3's serialiser closes the
round-trip test surface but isn't strictly required — Q4's
constructors can land first.

### Why this belongs in P54 scope

Without Q4, P54 ships a one-way JSON pipeline.  Users can *read*
structured data but can't *write* it — so a loft web service
answering a request with JSON, a test that wants to mock a
response body, or any system that composes JSON from loft values
hits a wall.  "General-purpose JSON support" is the explicit P54
goal; Q4 is required for that, not an extra.

---

## Active design — P54-U (unified JSON parser)

**Bite.**  After P54 step 5 + 6 + Q1 schema-side landed, two JSON
parsers coexist in the codebase, and they accept slightly
different dialects:

- **`src/json.rs::parse`** — schema-free, two-pass, RFC 8259
  strict.  Produces a `Parsed` enum tree consumed by
  `n_json_parse` (P54 arena materialiser) and `n_struct_from_jsonvalue`
  (Q1-aware schema walker).  Rejects bare-key objects like
  `{val: 7}` (only `{"val": 7}` accepted).
- **`src/database/structures.rs::parsing`** — schema-driven,
  single-pass.  Walks JSON text and writes directly into struct
  records via the database's known-type schema.  Lives behind the
  `OpCastVectorFromText` opcode used by `vector<T>.parse(text)`,
  `text as Type` casts, and the fallback in `parse_type_parse`
  for non-text non-JsonValue arguments.  Accepts BOTH standard
  RFC 8259 JSON AND loft-native bare-key syntax (`{val: 7}`,
  `{name: "x"}`).  Production-tested for years.

The dialect drift is the user-visible symptom: the same loft
program parsing the same text via `User.parse(text)` (auto-wrap →
strict) versus `vector<User>.parse(text)` (legacy → lenient)
applies different acceptance rules.  The doc comment in
`tests/scripts/57-json.loft::test_json_parse_loft_native` already
notes the lenient form was renamed to use standard JSON when the
auto-wrap path was wired — but the legacy parser still accepts
either form transparently.

**Decision: one parser, two modes.**

A unified parser exposes a `dialect: Dialect` parameter
(`Dialect::Strict` / `Dialect::Lenient`).  Strict mode is RFC
8259 verbatim — bare keys rejected.  Lenient mode also accepts
loft-native unquoted identifier keys.  All other features (number
syntax, string escapes, structural punctuation, depth handling,
RFC 6901 path tracking, line:col tracking, context-snippet
diagnostics) are identical between modes.

**Critically: the current data-import path stays unchanged.**
The lenient-mode acceptance set is a strict superset of the
strict-mode set, AND a strict superset of what `structures.rs`
accepts today.  No `.loft` file or `.txt` data file that parses
today stops parsing under the unified parser — the lenient mode
is the new default for legacy entry points.

### Mode selection

| Entry point | Default mode | Rationale |
|---|---|---|
| `json_parse(text) -> JsonValue` | Strict | RFC 8259 spec match; the typed JsonValue surface is for new code |
| `Struct.parse(text)` (auto-wrapped via `json_parse`) | Strict | Inherits json_parse's mode |
| `Struct.parse(json_parse(text))` | Strict | Same as above |
| `vector<T>.parse(text)` | Lenient | Preserves the existing data-import path |
| `text as Type` / `text as vector<T>` cast | Lenient | Preserves existing semantics |
| `Struct.parse(text)` direct (non-auto-wrap fallback) | Lenient | Preserves existing semantics |

A user who wants strict JSON for a vector parse explicitly opts
in: `vector<T>.parse(json_parse(text))` (once `vector<T>.parse`
accepts JsonValue alongside text — a small extension once the
unified walker covers `vector<struct>` end-to-end, which it
already does in P54 step 5).

### Surface changes (`src/json.rs`)

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// RFC 8259 strict — bare-key objects rejected.
    Strict,
    /// Strict + loft-native bare keys (`{val: 7}` ≡ `{"val": 7}`).
    Lenient,
}

impl Default for Dialect {
    fn default() -> Self { Dialect::Strict }
}

pub fn parse(input: &str) -> Result<Parsed, ParseError>;            // existing — Strict
pub fn parse_with(input: &str, dialect: Dialect) -> Result<Parsed, ParseError>;
```

The existing `parse(input)` keeps its signature (calls
`parse_with(input, Dialect::Strict)`) so all current callers stay
green.  New callers wanting lenient mode invoke `parse_with`.

### Bridging `OpCastVectorFromText` to the unified parser

The legacy `OpCastVectorFromText` body in `src/database/structures.rs`
gets reimplemented as:

```rust
pub fn parsing(stores: &mut Stores, text: &str, target_kt: u16) -> DbRef {
    // 1. Parse via unified parser, lenient by default to preserve
    //    legacy data-import compat.
    let parsed = match crate::json::parse_with(text, Dialect::Lenient) {
        Ok(p) => p,
        Err(e) => {
            // Legacy behaviour: zero-fill struct, push s#errors.
            stores.last_parse_errors.push(format_error(text, &e, 1, 1));
            return zero_struct(stores, target_kt);
        }
    };
    // 2. Walk the Parsed tree into the target struct/vector via a
    //    new helper that mirrors the JsonValue walker but consumes
    //    Parsed directly (no arena round-trip — Parsed lives only
    //    on the Rust stack).
    walk_parsed_into_target(stores, target_kt, &parsed)
}
```

The walker `walk_parsed_into_target` handles both struct and
vector targets (the latter for `vector<T>.parse(text)`'s wrapper-
struct shape).  It reuses the per-field-type dispatch matrix
already in `n_struct_from_jsonvalue` — extracted into a shared
helper that operates on either a `Parsed` ref or a JsonValue
DbRef.

After this, `src/database/structures.rs::parsing` shrinks from
~600 lines of hand-rolled scanner + dispatcher to ~50 lines of
parse-then-walk.

### Handling the dialect divergence carefully

A `.loft` test or data file parsed lenient today might also be
syntactically valid strict JSON (most are).  The migration
strategy:

1. **Add the Dialect enum + `parse_with` to `src/json.rs`** —
   pure addition, no behaviour change.  ✅ **Landed 2026-04-14**
   (`Dialect::Strict`, `Dialect::Lenient`, `parse_with(input,
   dialect)`; existing `parse(input)` is a shim over
   `parse_with(input, Dialect::Strict)`).
2. **Implement bare-key acceptance in `parse_object`** behind a
   dialect check.  Single conditional in the key-parsing
   branch.  ✅ **Landed 2026-04-14** (extracted
   `parse_object_key` helper; accepts `[A-Za-z_][A-Za-z0-9_]*`
   under `Dialect::Lenient`, rejects under `Dialect::Strict`).
3. **Reimplement `OpCastVectorFromText`'s `parsing`** to call
   `parse_with(text, Lenient)` + `walk_parsed_into_target`.
   ✅ **Landed 2026-04-14** — `Stores::walk_parsed_into` +
   `walk_parsed_struct` + `walk_primitive_into` in
   `src/database/structures.rs`.  Dispatches on every `Parts::*`
   variant (Base, Struct, EnumValue, Enum, Vector/Sorted/Array/
   Ordered/Hash/Spatial/Index, Byte, Short).  `Stores::parse`
   and `parse_message` route unified-first with legacy fallback
   gated for error-path position reporting.
4. **Verify** via the existing test scripts (`57-json.loft`,
   `58-constraints.loft`, `24-json.loft`) — every previously
   passing parse still passes.  ✅ **Verified 2026-04-14** —
   full `cargo test --release` pass (897/0 failed), plus
   instrumented `LOFT_P54U_TRACE` run showing zero success-path
   fallback hits across `issues` (437), `data_structures`
   (16), `wrap` (45), docs, and scripts.
5. **Delete** the now-unused scanner code in
   `src/database/structures.rs` (only the entry point and the
   Parsed-walker stay).  ✅ **Landed 2026-05-07** — the legacy
   hand-rolled scanner was already gone (Phase 2 walker
   superseded it organically).  Phase 3 work: cleaned up doc
   comments mentioning the "fallback" / "still kept for the
   transition", and added byte-offset threading through
   `walk_parsed_into` / `_struct` / `_primitive` so leaf
   type-mismatches report the field's real position via the
   `key_at` field already carried on `Parsed::Object` entries.
   The `"line N:M path:X"` shape via `format_walk_err` continues
   to back `tests/data_structures.rs::record` (`"line 1:7 path:blame"`).
   New regression: `p54u_leaf_mismatch_reports_field_position`
   in `tests/data_structures.rs` — locks the invariant that a
   primitive type mismatch inside a struct body reports a
   non-byte-0 position.

No public API changes.  No script-side migration required.  No
diagnostic regressions — both modes produce the rich Q1 errors
already shipped on the strict path.

### Implementation cost

- `src/json.rs`: ~30 lines (Dialect enum + parse_with + the
  bare-key conditional in parse_object).
- `src/database/structures.rs`: -540 lines (delete the hand-
  rolled scanner) + ~50 lines (parse-then-walk shim).
- New shared helper `walk_parsed_into_target`: ~120 lines
  (mirrors `n_struct_from_jsonvalue` but consumes `Parsed`).
- Tests: 3 new acceptance tests (bare-key accepted under
  Lenient, rejected under Strict; dialect-difference one-liner;
  legacy `text as Type` still works on a bare-key input).

### Why this belongs as a follow-up rather than a P54 sub-step

P54 already delivered the user-facing typed-JSON surface +
struct-from-JsonValue codegen.  The two-parser drift is an
internal cleanup — a user holding the typed `JsonValue` surface
can already parse, navigate, build, serialise, and unwrap into
structs.  The unification is about reducing maintenance surface
(one scanner instead of two, one dialect knob instead of two
divergent acceptance rules) and delivering Q1 diagnostics
uniformly across every text→JSON entry point.

### Unified diagnostic shape

Today three error sources produce three different formats:

| Source | Origin | Format example |
|---|---|---|
| Parser-side (`json.rs::format_error`) | Syntax error during `json_parse(text)` | `parse error at line N col M (byte B):\n  path: /a/b\n  <message>\n  <snippet with caret>` |
| Schema-side (`n_struct_from_jsonvalue` walker) | Type mismatch unwrapping JsonValue → struct | `User.age: expected JNumber, got JString` |
| Legacy (`s#errors` from `OpCastVectorFromText`) | Syntax or semantic error during `Type.parse(text)` | Free-form `format!()` strings — no consistent shape |

The unification step ships a single `Diagnostic` representation
that all three sources populate.  The text rendering degrades
gracefully when fields are missing — no diagnostic is worse for
the unification.

```rust
// src/json.rs (extends the existing ParseError into a richer
// shape that also carries schema-side info).
pub struct Diagnostic {
    pub kind: DiagnosticKind,
    /// RFC 6901 pointer accumulated through parser descent +
    /// (for schema errors) struct-field path.  `""` = root.
    pub path: String,
    /// Human-readable message.
    pub message: String,
    /// Source location — present whenever the diagnostic can be
    /// traced to original text bytes (parser-side always; schema-
    /// side iff the JsonValue arena tracks per-element source
    /// offsets, see Phase 2 below).
    pub location: Option<SourceLocation>,
    /// Type-mismatch detail (Schema kind only).
    pub expected: Option<String>,
    pub actual: Option<String>,
}

pub enum DiagnosticKind {
    Syntax,    // parser couldn't read the input
    Schema,    // walker found a kind/shape mismatch
    Conversion, // numeric over/underflow during extraction
}

pub struct SourceLocation {
    pub line: usize,
    pub column: usize,
    pub byte_offset: usize,
}

pub fn format_diagnostic(input: Option<&str>, d: &Diagnostic) -> String;
```

### Rendered forms

**Full info (parser-side syntax error):**
```
parse error at line 5 col 12 (byte 87):
  path: /users/3/age
  expected digit after `.`
    4 │       {"name": "Carol",
    5 │        "age": 1.}
                       ^
    6 │       },
```

**Schema mismatch with source location** (Phase 2, when arena
tracks byte_offset):
```
schema error at line 3 col 9 (byte 26):
  path: /users/1/age
  expected JNumber, got JString
    2 │   "users": [
    3 │     {"age": "twenty"}
                    ^
    4 │   ]
```

**Schema mismatch without source location** (Phase 1, when only
the JsonValue tree exists with no source-offset metadata):
```
schema error at /users/1/age:
  expected JNumber, got JString
```

**Cast (`text as Type`) failure** — same shape as parser-side:
```
parse error at line 1 col 8 (byte 7):
  path: /value
  unexpected character `,` after object key
    1 │ {value, 7}
             ^
```

### Single access surface

`json_errors()` returns the formatted diagnostic trail for ALL
sources (parser, schema, cast).  Each diagnostic renders into
the standard text shape above; the trail joins them with a
blank line separator (one blank line between blocks, no
trailing blank).

`s#errors` — the legacy per-record accessor kept for backward
compat — also resolves to the same trail, scoped to the record
constructed by the failing call.  No behavioural change for
existing callers; they get richer diagnostics for free.

### Path accumulation

The path is built incrementally through both parser and walker
phases:

- **Parser-side** (RFC 6901): pushed during `parse_object` /
  `parse_array` descent, popped on ascent.  Already shipped.
- **Schema-side** (struct walker): each recursion into a nested
  struct pushes `/<field_name>`; vector-element walks push
  `/<index>`.  When a diagnostic fires deep in the walker, the
  path captures the full descent.

Combined-path example: `Inbox.parse(text)` where `text` is
`{"users":[{"name":"A"},{"name":"B","age":"x"}]}` and User has
`age: integer`.  The schema diagnostic carries
`/users/1/age` — same RFC 6901 form parser-side errors use.

### Phase plan

**Phase 1 (with the parser-unification ship):** Introduce
`Diagnostic` + `format_diagnostic` + the trail accumulator.
Migrate all three sources to populate `Diagnostic` instead of
hand-rolling text strings.  Schema-side diagnostics initially
have `location: None` (no source-offset tracking yet) and render
as the "without source location" form.

**Phase 2 (follow-up):** Extend the JsonValue arena materialiser
to record the source byte offset for each element (one i32 slot
per record, ~12% memory overhead).  The walker reads these
offsets and populates `Diagnostic.location` so schema errors
also get line:col + context snippet.  Once shipped, every
diagnostic from every source has full location info.

### Why this design

- **No regression possible.**  The trail still gets populated
  the same way callers expect (json_errors trail + s#errors per-
  record).  The text gets richer.
- **Single source of truth for formatting.**  Adding a context-
  snippet style change happens in one function (format_diagnostic)
  — currently it would need to be duplicated across the three
  source paths.
- **Forward-compatible to structured access.**  A future Q-ticket
  could expose `JsonError` as a loft struct (`{ path: text, line:
  integer, column: integer, message: text, ... }`) so loft code
  can pattern-match on diagnostics rather than string-search.
  Same `Diagnostic` shape; just a new public surface.
- **Phase 1 is shippable independently** of arena-offset
  tracking.  The schema-side gets a consistent shape immediately;
  source location lands later when the arena tracks it.

### Tests for the diagnostic unification

- `p54_u_diagnostic_parser_format_unchanged` — the existing
  parser-side `q1_*` and `p54_err_*` tests still pass with
  the new `format_diagnostic` rendering the same text.
- `p54_u_diagnostic_schema_includes_path_and_kinds` — schema
  mismatch diagnostic includes the RFC 6901 path AND
  expected/actual variant names.
- `p54_u_diagnostic_cast_uses_same_shape_as_parse` — a `text as
  Type` failure renders identically to a `json_parse(text)`
  failure for the same input.
- `p54_u_diagnostic_trail_separator_format` — multiple errors
  render as separate blocks with a blank line between (not
  pipe-separated).
- `p54_u_diagnostic_path_combines_parser_and_walker_segments` —
  a schema error inside a deeply nested parsed structure shows
  the full `/<field>/<index>/<field>...` path.
- (Phase 2) `p54_u_diagnostic_schema_includes_source_location` —
  schema errors carry line:col + caret snippet once the arena
  tracks per-element byte offsets.

### Acceptance criteria

- `cargo test --release` — all suites green.
- `tests/scripts/57-json.loft::test_json_parse_loft_native` (the
  bare-key test that was renamed when auto-wrap landed) restored
  to bare-key form and passing under the Lenient default for
  vector parses.
- `src/database/structures.rs` line count down to ~250 lines
  (parse-then-walk shim + the existing struct/field-write
  helpers, which stay).
- `json_errors()` populates with the same RFC 6901 path + line:col
  + caret diagnostic for `text as Type` failures as for
  `json_parse(text)` failures.

### Tests

- `p54_u_lenient_accepts_bare_keys` — parser produces correct
  tree on `{val: 7}` under Lenient.
- `p54_u_strict_rejects_bare_keys` — parser returns ParseError
  on `{val: 7}` under Strict.
- `p54_u_text_as_type_still_accepts_bare_keys` — locks the
  data-import compat invariant.
- `p54_u_unified_diagnostic_for_cast` — `text as Type` failure
  produces the same Q1-format diagnostic as `json_parse` failure.

---

## Active design — Dep-inference for native fn returns (zero-leak unblock)

**Bite (2026-04-14).**  P54 ships a JsonValue surface
(`json_null`, `json_bool`, `json_number`, `json_string`,
`json_array`, `json_object` constructors plus `field`, `item`,
`kind`, `keys`, `fields`, `as_*` accessors).  Every chained
expression like `json_null().as_bool()` or
`v.field("x").kind()` leaks the temporary JsonValue store at
scope exit.  CI's debug-mode `execute_log_steps` assertion
(`Database X not correctly freed` at `src/state/debug.rs:994`)
catches it; release mode silently leaks per call.

Root cause: scope analysis's `inline_struct_return`
(`src/scopes.rs:~1026`) only lifts user-defined Reference returns
(`def.code != Value::Null`).  Native struct-enum returns
(`Type::Enum(_, true, dep)`) are never lifted — but the
constructors DO allocate fresh stores that need freeing.  The
existing system can't distinguish constructors (need lift) from
accessors (must NOT lift — they borrow into self's arena).

The discriminator is the `dep` field on the return type.  An
accessor borrows from `self` so its return should declare
`dep=[<self_attr_index>]`.  A constructor has no self so its
return should declare `dep=[]`.  Today both are declared
`dep=[]` because native function declarations never run through
`ref_return` (which only fires for fns with bodies).

**Status 2026-08-20 — shipped, and no longer observably load-bearing.**  The inference is
implemented (`src/parser/definitions.rs`, the `;`-terminated native-fn arm) and the bite
above is fixed: `json_null().kind()`, `v.field("a").item(1).kind()` and the rest neither
leak nor read freed memory, in loops, on `--interpret` and `--native`.

What could NOT be shown is that the inference is what fixes it.  Disabled (`if false &&`)
and rebuilt, every measurement is unchanged: the same values, the same store ledger
(1213 allocs / 1211 frees, peak 8 vs 9 live), clean under `LOFT_POISON=1` and
`LOFT_STORE_GUARD=1`, on both backends — and `loft_suite` still passes, so no existing
test covers its absence either.  The store-lifetime work that landed after this design was
written (@PLN85 and the `inline_struct_return` gates around it) appears to keep these
shapes in order on its own.

That is a measurement, not a verdict: "no shape I constructed distinguishes it" is weaker
than "it does nothing", and the probes were JsonValue-shaped because that is where the
surface is.  A native `self`-taking method on some OTHER struct-enum is the case most
likely to still need it.  Two things follow, and neither is a fix: the inference should not
be deleted on this evidence alone, and anyone who touches it should know the suite will not
tell them if they break it.

**Decision (2026-04-14): implicit dep inference for native fn returns.**

When a native function declaration `pub fn name(self: T, ...)
-> R;` is parsed and the return type R structurally matches the
self type T (same `Reference(d, _)` or `Enum(d, true, _)` with
the same `d`), automatically populate the return's `dep` with
`[<self_attr_index>]`.  No syntax change required; no per-fn
annotation; the parser infers borrowing from "returns the same
thing self is".

Cases handled correctly:

| Native | Self type | Return type | Inferred dep | Lifted? |
|---|---|---|---|---|
| `json_null()` | (none) | `JsonValue` | `[]` | YES (constructor, owned) |
| `json_string(text)` | (none) | `JsonValue` | `[]` | YES |
| `json_array(vec<JV>)` | (none) | `JsonValue` | `[]` | YES |
| `json_parse(text)` | (none) | `JsonValue` | `[]` | YES |
| `field(self: JV, text)` | `JsonValue` | `JsonValue` | `[0]` (= self) | NO (borrows) |
| `item(self: JV, integer)` | `JsonValue` | `JsonValue` | `[0]` | NO |
| `kind(self: JV)` | `JsonValue` | `text` | n/a (text) | n/a |
| `as_bool(self: JV)` | `JsonValue` | `boolean` | n/a (bool) | n/a |
| `Type.parse(text)` | (none) | `Type` | `[]` | YES |

The accessor-method tests added in P54 (`field()`, `item()`)
return JsonValue from a JsonValue self → infer dep=[0] → not
lifted → no use-after-free.  The constructor tests (`json_null`,
`json_bool`, etc.) return JsonValue with no self → dep=[] → lift
→ OpFreeRef fires at scope exit → no leak.

### Surface change (`src/parser/definitions.rs` or wherever
native fn parsing happens)

After parsing a native fn declaration with an empty body, before
storing the return type, check:

```rust
if let Type::Reference(ret_d, ref mut dep) | Type::Enum(ret_d, true, ref mut dep)
        = &mut def.returned
    && dep.is_empty()
{
    for (i, attr) in def.attributes.iter().enumerate() {
        if attr.name == "self" {
            let self_d = match &attr.typedef {
                Type::Reference(d, _) | Type::Enum(d, true, _) => Some(*d),
                _ => None,
            };
            if self_d == Some(*ret_d) {
                dep.push(i as u16);
            }
            break;
        }
    }
}
```

### Surface change (`src/scopes.rs::inline_struct_return`)

Once accessors carry a non-empty `dep` and constructors carry
`dep=[]`, extend the lift to native struct-enum constructors:

```rust
fn inline_struct_return(val: &Value, data: &Data, _outer_call: u32) -> Option<u32> {
    if let Value::Call(fn_nr, _) = val {
        let def = data.def(*fn_nr);
        // existing rule: user-defined struct return
        if def.name.starts_with("n_")
            && def.code != Value::Null
            && let Type::Reference(d_nr, _) = &def.returned
        {
            return Some(*d_nr);
        }
        // new rule: native struct-enum constructor (dep-empty)
        if (def.name.starts_with("n_") || def.name.starts_with("t_"))
            && let Type::Enum(d_nr, true, dep) = &def.returned
            && dep.is_empty()
        {
            return Some(*d_nr);
        }
    }
    None
}
```

### Tests to un-ignore once the dep-fix lands

All 34 entries in `tests/ignored_tests.baseline` tagged
`p54-leak: chained json call temp not freed (zero-leak gate)`
should pass once dep inference is correct AND the lift extends
to struct-enum constructors.  Iterate: regenerate baseline via
`python3 tests/dump_ignored_tests.py > tests/ignored_tests.baseline`,
run `cargo test --test issues p54_` in DEBUG mode, expect green.

### Acceptance criteria

- `cargo test --test issues p54_` — all p54 tests green in DEBUG
  build (no `Database X not correctly freed` panic).
- 34 ignore entries removed from `tests/ignored_tests.baseline`
  and from `tests/issues.rs` `#[ignore]` attributes.
- `tests/wrap.rs::loft_suite` — leak warnings on scripts
  `42-file-result.loft`, `62-index-range-queries.loft`,
  `76-struct-vector-return.loft` either disappear or are
  separately diagnosed (not all of them are this same root
  cause).
- 0.8.4 tag attempt resumes (per RELEASE.md § Safety gate
  deferral).

### Implementation cost

- ~30 lines in `src/parser/definitions.rs` for the inference.
- ~10 lines in `src/scopes.rs::inline_struct_return` to lift
  the new constructor case.
- One regression test in `tests/issues.rs` that asserts the
  inferred dep on `field()` and json_null() (read via
  `Data::def(...)`).
- ~5 deletions from `tests/ignored_tests.baseline` per
  unignored test (× 34).
- One CHANGELOG entry under `[Unreleased]`.

### Why this belongs here

This is the unblock for the 0.8.4 tag.  Without it, the P54
JsonValue surface ships with a real production leak — every
short-lived JsonValue (constructor or arena lookup) leaks
unbounded in any program that exercises the API.  RELEASE.md §
Safety gate explicitly blocks the tag on this.  Task #46
tracks the implementation.

---

## Compiler blockers — struct-enum bugs

**AUDIT 2026-05-21 — all B1-B7 closed on the interpreter.**  Ran the
full B-family regression set (`cargo test --release --test issues
p54_b` + `b7_`): **18 tests pass, 0 ignored**.  The `#[ignore]`
markers the historical notes below reference are gone; the biting
reproducers each verified live:

- **B2-runtime** (`s = Idle;` bare unit-variant in a mixed enum) —
  `p54_b2_unit_variant_literal_construction` + the qualified form pass
  on both backends.  The "layer 3 interpreter codegen NOT landed" note
  below is **stale** — it closed via the cross-PR struct-enum
  return-slot work.
- **B3** (struct-enum tail-expression / intermediate-local return) —
  `p54_b3_float_via_intermediate` etc. pass on the interpreter.  The
  "4-layer surgery, open" design below is **stale for the interpreter**.
- **B1/B5/B6/B7** — confirmed FIXED (as the notes already record).

**Native-only residual @P301 — FIXED 2026-05-21.**  The B3
intermediate-local form (`fn mk() -> JV { n = A{..}; n }`, and the
explicit `return n;` form) used to fail *native* compilation — the call
site emitted `n_mk(cell, ())` for a callee whose local was hoisted into
a hidden `DbRef` return-slot param (E0308).  The `p54_b3_*` guards are
interpreter-only, so this went unnoticed.  Root cause was NOT the call
site but `add_defaults` (`src/parser/mod.rs`): it had work-ref-allocating
arms for `Type::Vector`/`Type::Reference` hidden params but none for
`Type::Enum(_, true, _)`, so the hidden struct-enum arg stayed
`Value::Null` → `()`.  Fixed by adding the mirror `Type::Enum(_, true, _)`
arm; the call-site emitter then threads the work-ref automatically.  No
interpreter regression.  Cross-mode guard:
`tests/scripts/121-struct-enum-return-local.loft`.

The historical fix-design notes below are preserved as the narrowing
audit trail, not as current-state claims.

---

**Status (2026-04-13):** Concrete fix designs documented for all
four open compiler bugs (B2-runtime, B3, B5, B7) following an
explore-agent investigation.

The B7 single-line-fix prediction was tested and **did NOT close
the bug** — the type-match extension at `src/scopes.rs:1031` is
necessary but not sufficient; at least one other site in the
lifecycle machinery is also wrong.  Revised B7 estimate: **2-3
sessions** with `LOFT_LOG=full` instrumentation to pinpoint the
duplicate OpFreeRef emission.  Design + candidate sites listed in
§ B7 below.

The B5 / B2-runtime / B3 fix designs remain untested but
file:line targets are concrete.  Recommended landing order
restored to "B7 first, then B5, then …" because B7 still has the
largest blast radius even at the higher cost.

These bugs each surface any time a user writes a `Result<T, E>`-style
struct-enum, not just for JSON.  Fixing them unblocks the whole
`Option<T>` / `Result<T, E>` / planned coroutine-yield surfaces.

**B1 — Unit-variant match index-OOB.**  **FIXED** commit `61c36d7`.
Regression: `p54_b1_unit_variant_match_from_binding`.

**B2-runtime — Unit-variant literal construction in struct-enum
crashes.**  `JsonValue.JNull { is_null: true }` constructed at
runtime in a mixed enum doesn't produce a matchable value.
Workaround: build via the constructor path the parser uses; user
code avoids unit variants.  Test: `p54_b2_runtime_*` (`#[ignore]`).

**Fix design (original — stale; see revised note below).**
Unit variants in **mixed** enums (where some variants have fields
and some are unit) leave the payload buffer uninitialised when
constructed at runtime.  The variant tag byte is set correctly,
but the residual bytes beyond the tag carry whatever was on the
stack — match dispatch then reads garbage and either fails to
match or matches the wrong arm.

**Re-diagnosis 2026-04-13** (via `LOFT_LOG=full` on the same
`Sig { Off, Idle, On { level } }` reproducer): the observed
runtime symptom is **not** the predicted garbage-tag mismatch.
Instead, the test loops returning `value=16` thousands of times
until the harness's "Too many operations" guard fires at
`src/state/debug.rs:974`.  The match expression seems to
re-enter the function rather than exit it, which suggests a
codegen issue at the match-dispatch / return-slot layer, not
(only) the parse-time zero-fill.  Before attempting surgery,
capture a narrower trace with `LOFT_LOG=fn:run` and read
`parse_enum_field` → `parse_object` → match-arm return path
together.  Zero-filling the payload is likely necessary but
not sufficient.

**Partial fix landed 2026-04-13.**  Two root causes identified:

1. **Type-layout (LANDED):** `parse_enum_values` only added the
   "enum" discriminant attribute to struct variants (those with
   braces), leaving sibling unit variants with 0 attributes.
   `fill_database` then produced size-0 structures for them, and
   `Store::claim(size=0)` panicked "Incomplete record".  Fix in
   `src/typedef.rs::fill_all`: retroactively add the "enum" field
   to every unit variant whose parent's `returned` is a mixed
   `Type::Enum(_, true, _)`.  Off/Idle/On now all have the
   discriminant field in the native-schema emit.
2. **Bare-identifier construction (LANDED 2026-04-13):** extended
   `parse_constant_value` at `src/parser/objects.rs:481` to emit an
   inline `v_block` when the resolved variant's parent enum is
   mixed.  The block allocates a work-ref DbRef, calls
   `object_init` (which writes the discriminant via the "enum"
   field's default value), and returns the work-ref.  Work-ref is
   marked `skip_free` so only the receiving slot (var_s) frees the
   store at scope exit.  Native-emit verified: `let var_s: DbRef =
   { OpDatabase(__ref_1, 61); set_byte(0)=2; __ref_1 };` with
   single `OpFreeRef(var_s)`.

3. **Interpreter codegen (NOT landed):** `state::execute` still
   panics `Incomplete record` on the same reproducer, meaning the
   bytecode generation in `src/state/codegen.rs` doesn't observe
   the new `v_block` form the same way native-emit does.  Layer 1+2
   pass at the IR + native-Rust output level; the interpreter's
   bytecode emitter needs paired handling for
   `Type::Enum(_, true, _)` destination slots receiving a
   v_block containing an `OpDatabase` + field-init sequence.
   Follow `gen_set_first_ref_*` sites for the struct-Reference
   path and mirror for struct-enums.  Est. 1 session.

**Site:** `src/parser/objects.rs::parse_enum_field` (lines 1286-1314)
constructs the variant struct via `parse_object(e_nr, &mut cd)`.
For unit variants (0 attribute fields), the underlying
`OpDatabase` allocates a record but no field-init writes follow,
so the payload bytes stay garbage.

**Fix:** in `parse_enum_field`, detect the unit-variant case
(`def.attributes.is_empty()` for the variant struct) and emit a
zero-fill of the payload region after the `OpDatabase` /
`OpSetEnum` calls but before returning the value.  The payload
region size is `size(parent_enum) - 1` (everything after the tag
byte).  Reuse the existing bulk zero-fill op in `src/fill.rs` (or
add a 5-line `op_zero_bytes` handler if no exact op exists).

**Files:** `src/parser/objects.rs:1286-1314`; possibly a new op in
`default/01_code.loft` and `src/fill.rs`.
**Estimated scope:** one session.

**Verification path:**
1. `cargo test --release --test issues p54_b2_runtime_*` —
   2 currently-`#[ignore]`'d tests flip to green
   (`p54_b2_runtime_unit_variant_construction`,
   `p54_b2_runtime_qualified_unit_variant_in_mixed_enum`).
2. Full suite green.
3. Smoke: `JsonValue.JNull { is_null: true }` constructed at
   runtime in user code matches correctly via
   `match v { JNull { is_null } => ... }`.

**Side-effect risk:** low.  The fix narrows behaviour
(garbage-payload → zero-payload), making previously-undefined
match results well-defined.  Programs that accidentally relied on
the garbage value were already broken.

**B3 — Struct-enum tail-expression return crashes.**  Five
investigation sessions narrowed the diagnosis: needs **at least
4 coordinated codegen layers** changed (caller-side hidden-slot
allocation, `scopes.rs:307-318` hoist, `OpCopyRecord` deep-copy paths,
OpReturn discard accounting).  Single or even 3-layer attempts mutate
the symptom but never close it.  Workaround: explicit `return n;`
instead of `n` at function tail.  Tests: `p54_b3_*` (`#[ignore]`).
Estimated 8-12 source-line ranges across 2 files when attempted as
one focused refactor.

**Re-diagnosis 2026-04-13** (via `LOFT_LOG=crash_tail:30` on
`p54_b3_float_via_intermediate`).  The observed failure is not a
deep-copy / free-collision; it is `n_mk` **calling itself
infinitely** from its own tail position.  The tail expression `n`
(a local of struct-enum type) compiles to an `OpCall(fn=n_mk, …)`
each time — a fresh store is allocated (`ConvRefFromNull` →
`Database`) and the body re-executes before any return.  The heap
grows by one store per iteration until `free(): invalid next
size` aborts.

This sharpens the original 4-layer design: layer 1
(caller-side hidden-slot pre-alloc when the callee returns
`Type::Enum(_, true, _)`) is the site that currently mis-routes
the tail `n` load as a recursive call.  Without a reserved
return-slot the codegen falls back to the "call expression" path
for the tail local, and the return slot never materialises.
Landing layer 1 first, rerunning the trace, and only then adding
layers 2-4 is now the recommended order (instead of landing all
four together as the original design required).

**Fix design (original 4-layer, still applicable).**
Four coordinated layers must change.  Concrete file:line targets:

| Layer | File | Line(s) | Change |
|---|---|---|---|
| 1. Caller pre-alloc | `src/state/codegen.rs::generate_call` | 1410-1420 (before OpCall emission) | When callee's return type is `Type::Enum(_, true, _)`, emit `OpDatabase` for a 12-byte return slot, mirroring the Reference path |
| 2. Hoist | `src/scopes.rs` | 311 | Extend the hoist-set match from `Type::Reference \| Type::Vector` to also include `Type::Enum(_, true, _)` |
| 3. Deep-copy | `src/state/codegen.rs` | 827, 954-960, 975-1022, 1080, 1101, 1112-1130 | Every `Type::Reference` arm in OpCopyRecord-related match sites grows an `\| Type::Enum(_, true, _)` sibling |
| 4. Type extract | `src/state/codegen.rs::known_type` | 1761-1763 | Match arm currently extracts `Type::Reference(c, _) → c`; extend to `Type::Reference(c, _) \| Type::Enum(c, true, _)` |

**Estimated scope:** 2-3 sessions.  Each layer is independent and
testable; if a session lands only layers 1-2, the symptom mutates
but doesn't close — five investigation sessions confirmed all four
must land together.

**Verification path:**
1. After all 4 layers land: `cargo test --release --test issues p54_b3_*`
   — 4 currently-`#[ignore]`'d tests flip to green
   (`p54_struct_enum_explicit_return_of_local` already passes via
   the `return n;` workaround; the implicit tail-expression form is
   what the fix covers).
2. Full suite green.
3. Manual smoke: write the original BITING_PLAN reproducer
   (`fn mk() -> JV { A { v: 42 } }`) and confirm no crash.

**Side-effect risk:** medium.  OpCopyRecord deep-copy paths are
load-bearing for vector/struct passing; extending each match arm
needs a matching test for the new Enum case to avoid regressing
the existing Reference path.

**Why B3 sits *after* B7 in the recommended order:** they're
independent codegen surgeries with no overlap, and B7 unblocks
5x more downstream work per line of code touched.  B3 closes an
ergonomics gap; the `return n;` workaround stays good for any
user who needs it.

**B5 — Recursive struct-enum runtime crash.**  **FIXED.**  All four
guards (`p54_b5_recursive_struct_enum`,
`p54_b5_recursive_struct_enum_construction`,
`p54_b5_not_taken_arm_with_vector_binding_ok`,
`p54_b5_for_loop_over_enum_variant_vector`) now pass without
`#[ignore]`.  The recursive `count(Node {...})` returns 7 as
expected.  Layer 3 (the recursive tail-call return-PC bug
described historically below) closed as a side-effect of the
struct-enum return-slot work that landed across PR #168 → #174 —
no dedicated commit needed for layer 3 itself.

**Historical layered diagnosis kept for context.**  The reference
loft source:

```loft
pub enum Tree { Leaf { v: integer }, Node { kids: vector<Tree> } }
fn count(t: const Tree) -> integer {
    match t {
        Leaf { v } => v,
        Node { kids } => { c = 0; for k in kids { c += count(k); }; c }
    }
}
fn run() -> integer {
    root = Node { kids: [Leaf { v: 3 }, Leaf { v: 4 }] };
    count(root)
}
```

**Layer 1 — type registration (LANDED 2026-04-14).**  `fill_all`
now walks every struct and enum-variant attribute for
`Type::Vector(T)` fields and calls `data.vector_def(lexer, &T)`
before the main `fill_database` loop.  The wrapper struct
`main_vector<Tree>` is then registered and `fill_database` assigns
it a real `known_type`.  Parser-path assignment sites already
called `vector_def`; this covers the struct-enum-variant
declaration site that nothing else hit.  Closes the original
"Incomplete record" panic on `OpDatabase(db_tp=u16::MAX)`.

* Site: `src/typedef.rs::fill_all` (the pre-loop scan before
  line 215).
* Positive guard: `p54_b5_recursive_struct_enum_construction` in
  `tests/issues.rs`.

**Layer 2 — match-arm binding lifetime (LANDED 2026-04-14).**
`src/parser/control.rs:1103` `create_unique("mv_<field>", &field_type)`
now calls `self.vars.set_skip_free(v_nr)` on the binding variable.
The binding is a borrowed view (a `DbRef` field extraction from
the subject's record) — it does not own a store.  Without
`skip_free`, scope cleanup emitted `OpFreeRef(mv_…)` at function
exit.  In the taken arm, that decrements a store the binding
doesn't own.  In the **not-taken** arm, that slot was never
assigned and the free reads garbage bytes as a DbRef — observed
as out-of-bounds `store_nr ≈ 4621` in `Stores::free_named`.
Closes the garbage-FreeRef crash.

* Site: `src/parser/control.rs:1103-1125`.
* Positive guards: `p54_b5_not_taken_arm_with_vector_binding_ok`,
  `p54_b5_for_loop_over_enum_variant_vector` in `tests/issues.rs`.

**Layer 3 — recursive tail-call return PC (OPEN).**  After layers
1 and 2 land, the still-ignored test `p54_b5_recursive_struct_enum`
now gets FURTHER through execution before crashing.  The full
construction + match + for-loop path runs correctly until the
inner recursive `count(k)` call returns.  At that point the
trace shows:

```
4506:[160] GotoWord(jump=4643)                 ← jump to match end of inner call
4643:[160] Return(ret=9[128], value=4, discard=44) -> 3[116]  ← inner Return
   9:[120] Goto(jump=32)                       ← PC=9 is wrong; wanders away
  31:[120] CastIntFromText(v1=<raw:0x0>[104])  ← reads null text, wanders further
```

The inner Return pops `ret=9` from the stack as the return PC,
but PC=9 is nowhere near the caller's `c += count(k)` site — it
lands in unrelated bytecode (`OpCastIntFromText` on a null text),
then wanders into random ops.  The return-PC slot was read from
the wrong address.

**Candidate root cause.**  `src/state/codegen.rs::add_return`
around line 1772-1774 emits OpReturn with `self.code_add(self.arguments)`
— a per-function argument-frame size captured on `State`.  The
observed `ret=9` doesn't match n_count's actual argument frame
(1 × `const Tree` = DbRef = 12 bytes).  Either:

1. **`self.arguments` is stale** at the emit site — it's a `State`
   field reset per-function in `def_code` (`src/state/codegen.rs:57-79`)
   but not captured into the `Stack` context, so if something
   mutates it between function start and `add_return`, the value
   is wrong.  Mitigation: capture into `Stack` at function entry;
   use captured value in `add_return`.
2. **Ret-field semantics don't match "arg size"** — it may be the
   return-slot offset from the frame base.  Compute `ret_slot`
   explicitly at emit time rather than piggy-backing on
   `self.arguments`.
3. **Runtime reader mis-reads** — if emission is correct,
   `src/state/mod.rs:476-495` (`fn_return`) reads PC from the
   wrong stack offset.  The fix lives there.

**Fix path.**  Instrumentation-first: add a debug `eprintln!` in
`add_return` logging `(fn_name, self.arguments, size_of_return,
stack.position)`; correlate with the runtime trace's OpReturn
fields to disambiguate the three candidates before editing.

**Files:** `src/state/codegen.rs:1759-1778` (`add_return`),
`src/state/codegen.rs:57-79` (`def_code` prologue),
`src/state/mod.rs:268` (fn_call PC push),
`src/state/mod.rs:476-495` (`fn_return` PC pop).
**Estimated scope:** 1-2 sessions once the instrumentation
disambiguates which candidate is the actual root cause.

**Verification path:**
1. Instrumentation trace agrees with ONE of the three candidates.
2. The emitted OpReturn's `ret` field matches `sizeof(DbRef) = 12`
   for `const Tree`.
3. `p54_b5_recursive_struct_enum` un-ignored; output = 7.
4. The three positive guards (`..._construction`,
   `..._not_taken_arm_with_vector_binding_ok`,
   `..._for_loop_over_enum_variant_vector`) remain green.

**Related symptom.**  Layer 3's trace matches B3-family
(struct-enum tail-expression return).  B3 itself shipped 2026-04-13
as "struct-enum return types now get hidden caller pre-alloc args
just like Reference/Vector"; layer 3 may require pairing the call-
site pre-alloc with a recursion-aware return-slot accounting fix.

**B6 — Match-arm type unification.**  **FIXED** commit `5684df2`.
Regression: `p54_b6_match_arm_value_text_unifies`.

**B7 — Native-returned temporary lifecycle.**  **FIXED.**  All five
B7 regression guards pass without `#[ignore]`:

- `b7_method_on_jsonvalue_returning_integer_works` — `len(v)` on
  `JsonValue` (the original method-dispatch case).
- `b7_method_on_q4_constructed_jsonvalue_works` — same shape with
  the JsonValue built via `json_*` constructors.
- `b7_repeated_method_dispatch_on_jsonvalue_works` — chained
  method calls.
- `b7_multiple_json_parse_via_match_works` — sequential
  `json_parse` calls consumed via pattern match.
- `b7_character_interpolation_return_crashes` — name kept for
  search-back compatibility but the test passes (`build_b7c() == "h"`).

Closed as a side-effect of the B2-runtime retrofit, B5 layers 1+2,
the `t_9JsonValue_*` method-alias registrations, the dep-inference
fix in PR #171, and the lock-args-around-OpCopyRecord work in
PR #172 — no dedicated B7 commit was needed.

The historical paragraphs below describe the bug's reach at the
time they were written — preserved as the narrowing audit trail,
not as current-state claims.

**Unification finding 2026-04-13.**  B7's signature — `~500 iterations of
Return(ret=0, value=16, discard=0) at PC=0` followed by legitimate code
resuming, followed by store-leak warning + double-free — **matches B2-runtime
and B3 trace-for-trace**.  All three fire `OpReturn` / `OpCall` in a loop at a
function boundary involving a struct-enum value.  Specifically:

* B2-runtime: `s = Idle; match s { ... }` — OpReturn loops after the match.
* B3: `fn mk() -> JV { n = A{..}; n }` — OpCall loops (function calls itself).
* B7: `len(v_b7m)` where `v_b7m: JsonValue` — OpReturn loops after len.

The common thread: the **caller's reserved return slot** is wrong-sized or
wrong-addressed for a `Type::Enum(_, true, _)` value.  OpReturn pops `value=16`
bytes (the DbRef size) but the stack pointer advances because the caller
reserved the slot incorrectly; eventually the stack unwinds to a non-zero PC
and normal code resumes.  A **single fix** to the return-slot reservation /
OpReturn accounting for struct-enums likely closes all three items together.

Scope analysis (`src/scopes.rs`) doesn't emit `OpFreeRef` correctly
for the JsonValue store returned by `json_parse`.  The store leaks
on a chain of method calls AND any subsequent method-call site
trips a double-free at exit even when the method does no
allocation of its own.  Confirmed symptoms:

- `n_json_parse` returning a string variant + `as_text()` →
  caller's text-return path frees the JsonValue store before the
  text copy completes (`free(): invalid next size` at exit).
- Chained JSON access (`v.field("a").item(0).field("b")`) leaks
  intermediate stores.
- `fn f() -> text { c = txt[0]; "{c}" }` SIGSEGVs (discovered
  while writing INC#9 regression tests) — same family:
  native-returned text temporary built via `n_format_text` on a
  character isn't tracked for free on the outer function's
  return path.
- **(new, found 2026-04-13)** ANY method call on a JsonValue
  local crashes — even a method that just reads the discriminant
  byte and returns an integer (`len(v)`).  The crash is exit-time
  double-free, but the test harness sees it as SIGSEGV before
  reporting the function's return value.  Discovered while
  attempting to ship Q2's `kind(v) -> integer` peek; reverted
  the ship and parked the regression guard at
  `b7_method_on_jsonvalue_returning_integer_crashes` (`#[ignore]`).
- **(new symptom — INC#9 caveat)** `fn f() -> text { c = txt[0]; "{c}" }`
  SIGSEGVs.  The text built via `n_format_text` on a character
  isn't tracked for free on the outer function's text-return
  path.  Regression guard: `b7_character_interpolation_return_crashes`
  (`#[ignore]`'d).

**Retraction** (2026-04-13): an earlier note claimed "a second
`json_parse` call in the same function corrupts memory."
Investigation while writing B7 regression tests showed that
multiple `json_parse` calls work fine when each result is
consumed via pattern matching — the corruption observed in
earlier smoke tests came from the subsequent `kind()` / `len()`
method calls, not from `json_parse` itself.  Guard for the
working multi-parse path: `b7_multiple_json_parse_via_match_works`.

**Blast radius**: the entire `(JsonValue) -> T` method surface is
gated on this fix, not just text returns.  This means **Q2**
(`kind`, `keys`, `fields`, `has_field`), **Q3** (`to_json`,
`to_json_pretty`), the planned step-4 implementations of
`field`/`item`/`len`, and parts of step 5 (`Type::parse(JsonValue)`)
all sit downstream.

**Fix design (added 2026-04-13 from explore-agent investigation).**
The bug is in `src/scopes.rs::inline_struct_return` at line **1031**:

```rust
if let Value::Call(fn_nr, _) = val {
    let def = data.def(*fn_nr);
    if def.name.starts_with("n_")
        && def.code != Value::Null
        && let Type::Reference(d_nr, _) = &def.returned   // ← only Reference
    {
        return Some(*d_nr);
    }
}
None
```

The Set path at `scopes.rs:447-449` and `needs_pre_init` at line
1043 already accept `Type::Enum(_, true, _)`.  Only this lifting
site was missed — so native calls returning struct-enum (e.g.
`json_parse(...) -> JsonValue`) bypass lifting, the JsonValue
store is embedded in the argument frame, and the callee's exit
frees the store before the caller's `OpFreeRef` would have fired.

**Single-line fix (proposed by the design):**

```rust
&& let Type::Reference(d_nr, _) | Type::Enum(d_nr, true, _) = &def.returned
```

**Update (2026-04-13, after attempted ship):** the single-line fix
was applied and the test `b7_method_on_jsonvalue_returning_integer_*`
*still crashed* with the same "stores not freed" + "double free or
corruption" pattern, in both the inline form
(`len(json_parse(...))`) and the assigned form
(`v = json_parse(...); len(v)`).  The fix was reverted.

The type-match was demonstrably incomplete (the Set path and
`needs_pre_init` already accept `Type::Enum(_, true, _)` —
`inline_struct_return` was the only outlier), but **necessary is
not sufficient**.  At least one other site in the lifecycle
machinery must also be wrong.  Candidates to investigate next:

1. `n_json_parse` may be allocating with the wrong initial
   ref-count — `stores.database()` returns a fresh store; if the
   initial ref-count is 1 but the caller's `OpFreeRef` is also
   wired to decrement, an unrelated path may also be issuing a
   free.  The original P54 design plan (Step 1) called this out
   as the B7 root cause: "allocate the arena store inside the
   caller's variable's store, not via `stores.database()`".

2. ★ **Most likely candidate (narrowed 2026-04-14 via explore-agent
   walk of `src/scopes.rs`):** the **Set-path at lines 447-466**
   marks the `__ref_*` *temporary* binding `skip_free` when a
   native returns a struct-enum, but **does not mark the
   receiving variable `v` itself**.  Then at scope exit,
   `get_free_vars` at line 759 evaluates
   `emit = dep.is_empty() && !in_ret && !function.is_skip_free(v)`
   — since `v` was never marked, the check returns true and a
   second `OpFreeRef(v)` fires on top of the callee's internal
   free.  **Fix:** extend the Set-path marking logic to also call
   `self.vars.set_skip_free(v)` on the LHS receiving variable
   when its origin is a `Type::Enum(_, true, _)` native return.
   Mirror of the existing temporary-marking code path.

3. The interpreter codegen `state/codegen.rs:1043-1050`
   (referenced in the line 445 comment as the sibling skip-free
   logic) may need parallel treatment — only if candidate 2
   alone is insufficient.

**Estimated scope (revised):** 2-3 sessions.  Not the one-line
fix the design predicted.  Session 1: instrument the run with
`LOFT_LOG=full` for `b7_method_on_jsonvalue_returning_integer_crashes`
and confirm candidate 2's double-emit hypothesis (log every
`OpFreeRef` emission site along with the variable number).
Session 2: ship the `set_skip_free(v)` extension + the
single-line `inline_struct_return` fix together; re-run, confirm
the single-store free; un-ignore the two B7 guard tests.
Session 3: un-ignore the 5 P54 text-return-through-fn family
tests + verify `b7_multiple_json_parse_via_match_works` stays
green.

**Verification path:**
1. Run the currently-`#[ignore]`'d B7-family tests with `--ignored`
   and confirm they flip to passing:
   - `b7_method_on_jsonvalue_returning_integer_crashes`
   - `b7_character_interpolation_return_crashes`
   - `p54_extractor_as_text` and 3 sibling text-return tests
   - `p54_missing_chain_returns_jnull`
2. Then unignore them.
3. Confirm `b7_multiple_json_parse_via_match_works` (currently
   passing) stays green — guards the working multi-parse path.
4. Full suite green.

**Side-effect risk:** low.  Lifting was proven safe for
References in the P135 fix; extending to Enums preserves the
invariant (native function allocates and owns the store, lifted
temp takes ownership, OpFreeRef frees once at scope exit).  No
ref-count machinery involved.

**One fix turns 8 things green together**: 5 ignored P54 tests
(the text-return-through-fn family + chained-access) + 2
B7-prefixed guards + the INC#9 character-interpolation crash.
Highest-leverage compiler bite remaining and the bottleneck for
nearly every JSON deliverable on the roadmap.

---

## Enhancement tiers

Quality investments ranked by leverage.  Pick **one Tier 1** as the
multi-session sprint, pair with **one Tier 2** as a
session-of-the-week background bite.

### Tier 1 — closes whole classes of bugs

> **⛔ HISTORICAL — all three items have SHIPPED (items 1–2 checked 2026-07-10; item 3 closed
> 2026-08-20).**  B7 closed with the B2–B7 audit (2026-05-21, both backends); C54 (`integer` → i64)
> landed 2026-04-21 via @PLAN01 / @PLN88; the ignored-test baseline reached zero known gaps
> 2026-08-20.  Kept as a worked example of the "closes a whole class" selection criterion, which
> still applies — today it selects **Cluster C / H10**
> ([STABILITY_ROADMAP.md](STABILITY_ROADMAP.md)).

1. **B7 lifecycle for native-returned struct-enum temporaries.** ✅ CLOSED 2026-05-21.
   Unblocks 5 P54 ignored tests in one fix.  Scope analysis pattern,
   precedent in `File`'s ref-count handling.

2. **C54 integer → i64.** ✅ LANDED 2026-04-21.  Eliminates the `i32::MIN` sentinel trap
   that has spawned three documented gotchas.  Multi-session,
   sub-tickets land independently (see § C54).

3. **Drive `#[ignore]`'d tests to zero.**  ✅ **CLOSED 2026-08-20 — no known gap is
   parked behind an `#[ignore]` any more.**  `tests/ignored_tests.baseline` is down to
   ONE entry, `regen_fill_rs`, which regenerates `src/fill.rs` from `default/*.loft` and
   is maintenance rather than a gap: it is meant to be run by hand when the stdlib
   changes, and running it in CI would test the generator against its own output.  The
   `#[ignore]`s left elsewhere in `tests/` are deliberate opt-in harnesses in the same
   spirit — the differential oracle (a rustc invocation per corpus program) and one
   `host_call` measurement that is explicitly "a measurement, not a gate".

   The last real gap was **`pln102_one_known_operand_forward_float_still_mistyped`**
   (closed 2026-08-20, below).  Worth keeping the shape of that close: the test's own doc
   comment predicted the fix needed "the operator search to defer on the RESULT type
   rather than on operand knownness — a larger change than the guard widening", and that
   prediction had gone stale.  It was written when the all-unknown restriction really was
   load-bearing; loft#918 then added a second deferral at the reject site and quietly took
   over the job the restriction existed to do.  Nobody re-measured, so the note kept
   sending readers away from a one-word fix (`all` → `any`) for months.  **A parked test's
   stated reason is a claim with a date on it, not a standing fact** — re-measure it before
   budgeting for the redesign it predicts.

   Sustainable cadence when new ones appear: 1–3 per session.

   **Closed 2026-08-20:** `pln102_one_known_operand_forward_float_still_mistyped` — a
   binary op with ONE unresolved operand (`f() - 1`, `f`'s type declared lower in the
   file) was refused, not mis-valued: pass 1 matched `OpSubInt` off the literal and locked
   the local to `integer`, and pass 2's real `float` return then raised *"Variable 'a'
   cannot change type from integer to float"* — a correct line about a decision the reader
   never made.  Writing the type down (`a: float = f() - 1`) was refused too, with the
   message reversed.  The first-pass deferral in `call_op` now fires when ANY operand is
   unresolved rather than only when all are.  Guarded both ways: four `pln102_*` tests in
   `tests/issues.rs` (each verified to FAIL with the predicate reverted, and only those
   four), the pre-existing
   `pln102_one_known_operand_keeps_the_mismatch_diagnostic` for the diagnostic path, and
   `tests/scripts/forward-operand-arithmetic.loft` for both backends.

   **Closed 2026-04-14:** `file_content_nonexistent_trace` — the
   un-ignored test now exercises the regular `execute` path's
   "missing file → empty text" guarantee.  The historical
   SIGSEGV applied only under `execute_log` (LOFT_LOG=full),
   not the regular runtime; the test as written hits the
   regular path and the empty-text contract is stable today.
   The execute_log SIGSEGV (misaligned-slot codegen issue in
   the stack allocator) is a separate, deeper bug that
   doesn't gate this regression guard.

   **Closed 2026-04-14:** `p122_long_running_struct_loop` — ignored
   only because the 10 000-frame × 10-brick struct-alloc loop takes
   ~10 min in debug mode; passes in ~0.05 s in release.  Converted
   the attribute from `#[ignore]` to
   `#[cfg_attr(debug_assertions, ignore = "…")]` so the test now
   runs automatically in `cargo test --release` (CI's default) and
   continues to skip in debug for day-to-day iteration.  Debug-only
   manual run still works via `cargo test --ignored`.

### Tier 2 — preventive, low-risk, high-readability

4. **~~`cargo clippy --no-default-features --all-targets -- -D warnings`
   cleanup.~~**  Landed 2026-04-13.  Full `--no-default-features`
   build now goes clean through clippy on both lib and bin targets
   and both feature combinations still compile identically.  Fix set:
   - **`src/parallel.rs`** — 12 original lints: 7× `needless_pass_by_value`
     and 2× `not_unsafe_ptr_arg_deref` suppressed via
     `#[cfg_attr(not(feature = "threading"), allow(…))]` (the value is
     consumed by `Arc::new(program)` in the threading branch, borrowed
     in the non-threading branch; making the public fn `unsafe` would
     cascade across every `par(...)` site).  3× `needless_range_loop`
     in the non-threading fallbacks refactored to
     `for (row_idx, r) in results.iter_mut().enumerate()`.
   - **`src/parallel.rs`** — cascaded `dead_code` on 5
     `run_parallel_*` fns + `WorkerPool::new`: same cfg-gated allow,
     the binary-crate view compiled by `main.rs` sees no callers
     under `--no-default-features`.
   - **`src/main.rs`** — `extract_toml_version`, `chrono_date`,
     `days_to_ymd` moved under `#[cfg(feature = "registry")]`; the
     `registry_sync()` tail-body (formerly reached only when
     `registry` is enabled) wrapped in `#[cfg(feature = "registry")]`
     to resolve the `unreachable_code` warning that fired after the
     cfg'd-out branch's unconditional `exit(1)`.
   - **`tests/data_structures.rs`** — the lone `index_deletions` test
     that uses `rand_pcg::Pcg64Mcg` gated behind
     `#[cfg(feature = "random")]`; its imports the same.
   - **CI** — `Makefile` `ci:` target now invokes
     `cargo clippy --no-default-features --all-targets -- -D warnings`
     alongside the default-features gate, so the ratchet stays
     green on every push.
   - **Regression guard** —
     `tests/doc_hygiene.rs::ci_target_runs_no_default_features_clippy`
     reads the Makefile and fails if the gate is ever removed.

5. **Migrate `Struct.parse(text)` → `json_parse(text) → match`** in
   `tests/scripts/57-json.loft` and `tests/docs/24-json.loft` once
   P54 step 5 lands.  Unblocks step 6 (the rejection diagnostic)
   and turns the tests into examples of the modern API.

6a. **~~Drop `code!()`'s duplicate-test emission.~~**  Landed
    (investigation) 2026-04-13 — turned out to be a false positive: the
    `duplicate_macro_attributes` warning and "same test name printed
    twice" output both traced to a single orphan `#[test]`
    attribute in `tests/issues.rs` left over from a test-block
    move.  The `code!()` macro is clean.  Removed the orphan; added
    `tests/doc_hygiene.rs::no_orphan_test_attributes_in_tests_issues_rs`
    so the next orphan is caught at test time, not via a
    misattributed warning.  No further action.

6b. **~~Drift guard for `#[ignore]`'d tests.~~**  Landed 2026-04-13:
    `tests/doc_hygiene.rs::ignored_tests_baseline_is_current` loads
    `tests/ignored_tests.baseline` (name + reason per ignored test,
    20 rows today) and fails with a +/- diff when the set drifts.
    Regenerator at `tests/dump_ignored_tests.py`.  Catches
    un-ignored-without-baseline-update, silently-added new
    `#[ignore]`, and reason-string edits.  Does *not* yet run the
    ignored tests themselves and diff pass/fail/panic-message —
    that heavier nightly `--ignored` diff is the remaining gap.

6c. **~~Surface method-vs-free suggestions in diagnostics (both
    directions).~~**  Landed 2026-04-13 in `src/parser/fields.rs` and
    `src/parser/mod.rs`:
    - **method→free** (original): when field access fails and a free
      function `n_<field>` exists whose first parameter is compatible
      with the receiver type, the diagnostic now reads
      `"Unknown field vector.sum_of — did you mean the free function
      `sum_of(…)` ? (stdlib declared `sum_of` as free-only; see
      LOFT.md § Methods and function calls)"`.  Tests:
      `inc08_sum_of_is_free_function_only` locks the hint wording;
      `quality_6c_unknown_field_without_free_fn_has_no_hint` locks
      specificity (a genuinely-misspelled field still gets the plain
      message).
    - **free→method** (follow-on, landed same day): when a free call
      `name(…)` fails and a method `t_<LEN><Type>_<name>` exists on
      some other type (typically the user passed a wrong-type
      receiver to a `self:` method via free syntax), the diagnostic
      now reads `"Unknown function starts_with — did you mean the
      method `x.starts_with(…)` on text? (stdlib declared
      `starts_with` as a method; see LOFT.md § Methods and function
      calls)"`.  Methods declared on multiple receivers (e.g.
      `is_numeric` on both `text` and `character`) are enumerated
      with `/`.  Site: `src/parser/mod.rs::call` uses
      `find_method_receivers` to scan definitions for the
      `t_<LEN><Type>_<name>` pattern.  Tests:
      `quality_6c_free_call_on_wrong_type_suggests_method`,
      `quality_6c_free_call_lists_all_method_receivers`,
      `quality_6c_free_call_unknown_fn_has_no_method_hint` (negative
      — a genuinely-unknown name still prints the plain message).

6d. **~~Better errors for keyed-collection construction.~~**  Landed
    2026-04-13 in `src/parser/fields.rs::index_type`: the
    `"Indexing a non vector"` diagnostic now spells out both the
    missing feature (no generic-constructor expression) and the
    idiom that works (struct-field declaration + vector-literal
    initialisation).  Tests: `quality_6d_keyed_collection_constructor_hint`
    locks the new wording on the `hash<Row[id]>()` reproducer;
    `tests/parse_errors.rs::index_non_indexable` updated to the
    new text on its `v = 5; v[1]` baseline.  Implementing the
    generic constructor itself is a separate, larger task — not
    this diagnostic fix.

6. **Document one inconsistency per session.**  Following the
   INC#3 / INC#12 / INC#26 / INC#29 pattern — write the gotcha into
   LOFT.md, lock the behaviour with 2-3 regression tests.  INC#2
   (vector-vs-keyed-collection API gap), INC#8 (method-vs-free-function
   stdlib choice), INC#18 (`x#break` labelled-break syntax), and INC#27
   (no `x#continue` counterpart — silent bare-continue) all landed
   2026-04-13.  No further INC doc-bite candidates remain; future
   sessions should draw from Tier 1 or Tier 3 backlog items.

### Tier 3 — structural, larger payoff

7. **~~Bytecode cache verification.~~**  Landed 2026-04-13 in
   `tests/bytecode_cache.rs`.  `.loftc` shipped in commit `4039490`;
   the hit / miss / invalidation cycle is now locked with four
   process-level tests that drive the real `loft --interpret` binary
   end-to-end:
   - `first_run_writes_loftc_with_magic_header` — fresh compile
     creates `.loftc` next to the source, beginning with the `"LFC1"`
     magic bytes.
   - `second_run_reuses_cache_bytes_unchanged` — two consecutive runs
     on the same source leave `.loftc` byte-identical (hit path).
   - `source_change_invalidates_and_rewrites_cache` — editing the
     source changes the SHA-256 key; `.loftc` is rewritten and the
     new stdout reflects the new source (not a stale cached image).
   - `missing_loftc_is_recreated` — deleting the cache file between
     runs forces regeneration on the next run.

8. **~~Const store mmap path on Linux.~~**  Closed as
   deferred-by-design 2026-04-14.  [CONST_STORE.md § Phase B
   (mmap)](plans/82-const-store/README.md#memory-mapped-constant-store) reaches the
   opposite conclusion: at today's cache-file sizes (5-10 KB) mmap
   overhead (syscall + page tables) exceeds the memcpy savings, so
   the implementation path is intentionally not taken.  A benchmark
   here would lock in a micro-regression that the design has already
   ruled out.  If Phase C ever ships a large stdlib cache the
   tradeoff flips, at which point the benchmark becomes a useful
   companion to the mmap rollout — re-open then.

   In the meantime, the cache *load* path (not mmap-specific) is
   exercised end-to-end by the Tier 3 #7 bytecode-cache integration
   tests, so cache hit/miss correctness is locked even without a
   timing benchmark.  Regression guard —
   `tests/doc_hygiene.rs::quality_const_store_mmap_matches_const_store_md`
   asserts the two docs don't silently drift back out of sync.

9. **~~WASM FS bridge.~~**  Landed 2026-04-14 as STUBS, and superseded
   2026-08-11 by a real one (loft#851).  The stubs were the right answer
   while `--html` had no reachable filesystem: they made `file("x")`
   answer "absent" reliably instead of depending on what `std::fs` does
   in a given JS embedding.  `--html` now binds an actual filesystem over
   raw `loft_io` imports, so the stubs are gone and the file operations
   ask one question — the `host_fs` cfg (`build.rs`) — instead of a
   hand-written `feature = "wasm"` per site.  See
   [WASM.md § The page filesystem](WASM.md).  Tests:
   - `tests/html_wasm.rs::html_page_has_a_filesystem` and
     `::html_page_filesystem_cursor_matches_the_other_backends`
     — end-to-end over a real `--html` page, with every expected value
     taken from what `--interpret` and `--native` print for the same
     program, so it fails both on losing the filesystem and on growing
     one that answers differently.
   - `tests/html_wasm.rs::q9_html_file_content_returns_empty_on_wasm`
     — the half most easily broken by binding a filesystem: a path
     nobody wrote must still read as null, and "absent" and "an empty
     file" cross the bridge as one import.
   - `tests/html_wasm.rs::html_page_filesystem_unit_checks`
     (`tools/loft_fs_unit.mjs`) — the base tree and the reload, which a
     node-hosted page cannot reach.
   - `tests/doc_hygiene.rs::file_operations_gate_on_host_fs_not_the_wasm_feature`
     — static guard, and the reason the old one existed: these arms
     drift.  They already had, for a year — the interpreter's browser
     branches were real bridges while `codegen_runtime.rs`'s were stubs,
     so `--html` (which runs the generated code) had the empty
     filesystem and nothing said so.
   Separately, the native-only `file_content_nonexistent_trace`
   SIGSEGV under `execute_log` (called out in the test's own
   comment) is a misaligned-slot codegen issue in the stack
   allocator — unrelated to the WASM bridge and tracked
   independently.  The ignored test stays ignored.

### Tier 4 — process / hygiene

10. **PROBLEMS.md Quick-Reference is the source of truth — keep it
    that way.**  Three docs (Quick-Reference, long-form section,
    CAVEATS.md) drift independently and required two
    "doc hygiene" commits this sprint.  Either canonicalise one and
    have the others link, or add a `make docs-check` script that
    greps for FIXED markers in the long form and complains when the
    Quick-Reference still says open.
    - **Landed 2026-04-13:** `tests/doc_hygiene.rs` now guards all
      four sources — INCONSISTENCIES.md (Status blocks ↔ Resolved
      table), PROBLEMS.md (Quick-Reference ↔ long-form
      `### ~~N~~ FIXED` headings), CAVEATS.md (long-form
      `### ~~CX~~ DONE` ↔ Verification-log table), and QUALITY.md
      itself (main open-issues table must contain no crossed-out
      rows; Tier-2 strikethrough items must carry a `Landed
      YYYY-MM-DD` marker in their body).  Caught five existing
      drifts on first runs: #135 (PROBLEMS Quick-Reference), P137,
      C58/P135, and C60 (CAVEATS Verification-log), plus 6a's
      missing landing marker (QUALITY self-guard) — all corrected
      in the same commits.  Item 10's scope is now closed; future
      drift gets caught in CI instead of sprint-hygiene commits.

11. **~~Memory of recent decisions.~~**  Landed 2026-04-13.  Both
    PLANNING.md and PROBLEMS.md now open with a "Before
    proposing/opening …, check [DESIGN_DECISIONS.md]" paragraph in
    their intro — visible above the fold, not buried in the
    cross-references list at the bottom.  PLANNING.md's version
    targets feature proposals; PROBLEMS.md's version targets new
    bug reports (with pointers to C3 / C38 / C54.D as the classic
    re-opens).  Regression guard —
    `tests/doc_hygiene.rs::planning_and_problems_link_to_design_decisions`
    asserts both files mention `DESIGN_DECISIONS.md` in their first
    80 lines, so a future cleanup that strips the intro can't
    silently re-hide the register.

12. **~~A `make ship` target.~~**  Landed 2026-04-13.  `Makefile`
    now defines `ship:` as the canonical pre-push gate.  Four
    invariants chained with `&&` so the first failure aborts and a
    subsequent `git push` never runs:
    1. `cargo fmt --all -- --check` — formatting.
    2. `cargo clippy --all-targets --all-features -- -D warnings` —
       CI's exact Clippy invocation (2026-06-13: was
       `--release --all-targets`; a local pass with the narrower
       variant + a remote fail cost a full CI round, so `ship`
       mirrors the CI job verbatim now — `make gate` runs the same
       lint without the test suite for fast pre-push iteration).
    3. `cargo clippy --no-default-features --all-targets -- -D warnings`
       — the `--no-default-features` ratchet from #4 (previously easy
       to forget).
    4. `cargo test --release` — full suite.

    Distinct from `ci:` which optimises for the remote pipeline
    (logs to `result.txt`, runs GL + packages suites).  `ship` streams
    to the terminal and is the intended `make ship && git push`
    workflow.  Regression guard —
    `tests/doc_hygiene.rs::ship_target_chains_all_required_gates`
    reads the Makefile and asserts all four fragments appear in
    order, chained with `&&`.

---

## Recommended landing order

> **⛔ HISTORICAL — do not plan from this section (checked 2026-07-10).**  Every item it orders
> (B7, B5, B2, B3, C54, P54 steps 4–8) has since **shipped**: B2–B7 audited + closed 2026-05-21 on
> both backends, C54 landed 2026-04-21, P54 steps 4/5/6 complete.  Kept for the investigation
> record — the *method* (explore-agent → file:line targets → tested prediction) is still the model.
> The live ordering lives in [ROADMAP.md](ROADMAP.md) and [STABILITY_ROADMAP.md](STABILITY_ROADMAP.md).

**Updated 2026-04-13** — explore-agent investigation produced
concrete file:line targets for all four compiler bugs.  The B7
single-line-fix prediction was tested same day and did not close
the bug; revised estimate **2-3 sessions** (the type-match
extension is necessary but not sufficient — needs paired
investigation of the duplicate-OpFreeRef site).  B5 reclassified
from "needs arena workaround forever" to "one-session
memoization fix in `fill_database`".  Order rebuilt around these
findings.

1. **B7 — 2-3 session lifecycle fix** starting at
   `src/scopes.rs:1031` plus paired sites yet to be identified
   via `LOFT_LOG=full` instrumentation.  Still highest-leverage
   compiler bite — unblocks 5 ignored P54 tests + 2 B7-prefixed
   guards + the INC#9 character-interpolation crash + every
   `(JsonValue) -> T` method call.
2. **Q2** — `kind` / `keys` / `fields` / `has_field` natives
   become trivially shippable post-B7.  One session.
3. **B5 — memoise `fill_database`** in `src/typedef.rs`.  Removes
   the arena-indirection compulsion from P54 step 4 and unblocks
   future stdlib enums with recursive variants (Tree<T>,
   Result<T, E>, etc.).  One session.
4. **P54 step 4** — array/object materialisation.  Simpler
   post-B5 (natural `vector<JsonValue>` works); Q3 + Q4 unlock.
5. **Q1 schema-side reuse** — when P54 step 5 lands,
   `Type::parse(JsonValue)` reuses the already-shipped
   `format_error` infrastructure for per-field path diagnostics.
6. **P54 step 5** — `Type::parse(JsonValue)` codegen with the
   field-type matrix + strict / permissive policy.
7. **Q4** — `json_null` / `json_bool` / … / `json_object`
   constructors.  Bypasses B2-runtime by allocating in Rust;
   ships any time after step 4.
8. **Q3** — `to_json` / `to_json_pretty` + `T.to_json()` codegen.
   Round-trip tests become possible.
9. **B2-runtime — zero-fill unit-variant payload** in
   `src/parser/objects.rs::parse_enum_field`.  Quality-of-life for
   any user constructing struct-enum literals at runtime; not a
   P54 blocker (Q4 bypass already works).  One session.
10. **B3 — four-layer codegen surgery** for struct-enum tail
    returns.  2-3 sessions.  Closes the implicit-return ergonomics
    gap; the `return n;` workaround stays good for any user who
    needs it.  Lower priority than items 1-9.
11. **P54 step 6** — sweep stdlib/tests off `Struct.parse(text)`,
    ship rejection diagnostic.
12. **P54 steps 7-8** — unignore remaining P54 tests; doc sweep.
13. **C54.A → C → B → E** — integer i64 widening.  Schedule last
    in 0.9.0 so earlier bites are fixed on the existing layout
    before the schema bump.

Tier 2 items run in parallel as session-of-the-week background
bites.  Tier 3 / 4 — at most one per release window.

---

## See also

- [PROBLEMS.md](PROBLEMS.md) — historical bug log (interpreter
  robustness, web services, graphics)
- [CAVEATS.md](CAVEATS.md) — verifiable edge cases with reproducers
- [INCONSISTENCIES.md](INCONSISTENCIES.md) — language design
  asymmetries
- [DESIGN_DECISIONS.md](DESIGN_DECISIONS.md) — closed-by-decision
  register
- [PLANNING.md](PLANNING.md) — priority-ordered enhancement backlog
- [ROADMAP.md](ROADMAP.md) — items grouped by milestone
- [DEVELOPMENT.md](DEVELOPMENT.md) — branching, commit order, CI
