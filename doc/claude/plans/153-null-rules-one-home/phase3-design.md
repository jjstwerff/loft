# @PLN153 phase 3 — the N-Store refusal at ONE point (design skeleton, 2026-09-05)

## The invariant (one sentence)

A value of type `τ?` reaching a slot of type `τ` without a discharge is REPORTED exactly
once, at the store, with the severity `(N-Store)` names: a WARNING where `τ`'s non-null form
reserves its null distinctly, an ERROR where it does not (narrow `u8`…`u32`).

## Re-assertion sites today (the census — five homes, three severities, two texts)

| # | site | fires on | severity | text |
|---|---|---|---|---|
| 1 | `parser/mod.rs` DN3 branch (`pln25_dn3_enabled`, :4178–4221) | field / return / element / arg via `nstore_check` | WARN full, ERROR narrow | "a nullable `τ?` is stored into {what} of the non-null type `τ` — it becomes null there …" |
| 2 | `parser/mod.rs` null-literal branch (:4240–4285) | bare `null` into a non-null slot | WARN (heap never escalates), ERROR narrow | "`null` is stored into {what} of the non-null scalar type …" |
| 3 | `parser/mod.rs:11711` call-argument gate (`callarg_nstore_enabled`) | `f(x?)` into `p: τ` | routes to #1 | — |
| 4 | `variables/mod.rs::change_var_type` | a DECLARED LOCAL `x: τ` assigned a `τ?` | **ERROR, full width too** | "Variable 'x' cannot change type from integer to integer?; discharge the null where it is produced …" |
| 5 | `parser/control.rs:1233 / :1305` (cited `@FR-N-Store`) | a tail / an `if` join delivering `τ?` to a `τ` return | (read the sites) | — |

Measured on the phase-1 pairs: a declared local → #4 ERROR (`153-n-store-refused-into-a-declared-local`);
an element → #2 WARNING (`153-n-store-warns-into-an-element`); a constant index into a literal →
silent (provably in range, legitimate); the narrow local → #4's text spells the alias
`integer(0, 255)` rather than `u8`.

## The formal inconsistency phase 3 must settle FIRST

`types.md:135 (N-Store)`: *"REJECTED — a WARNING for most τ … a hard ERROR only for narrow
widths"*.  `types.md:159` (the worked table): `a: integer = 2; a = v[i]` → **"type error"**.
The code does both: site #4 errors, sites #1/#2 warn.  Per formal/README's doctrine the rules
settle it, but here the rule and its own example disagree.  Reading: the RULE line is the
contract (it is what @PLN102's null-flow cutover shipped and documented as the WARNING split);
the table row is the pre-cutover reading that was never updated — the same "auto-`τ?` reading
above is superseded" note types.md:150 carries for N-Parse.  So: a deviation entry (types.md
has no history file — check) that the declared-local site is an ERROR where the rule says
WARNING, closed by phase 3's fold; and the table row corrected to "warning" with the
COMPATIBILITY question (open design question 3) left where it is.

⚠ Before deciding, MEASURE what a WARNING at site #4 would change: `x: integer = v[i]` today
refuses; after, it compiles with a warning and `x` holds null at run time — the (N-Store)
semantics ("the store proceeds, the slot holds the null sentinel").  That is a behavioural
widening (more programs accepted), which COMPATIBILITY.md permits in this direction.

## Where the ONE point is (open design question 1) — two candidates, one probe each

**A. `Parser::convert`'s Optional-SOURCE arm** (parser/mod.rs ~4470: `if let Type::Optional(inner)
= is_type { return self.convert(code, inner, should) }`).  Every store passes through `convert`
(`⇐` = `convert(code, is, should)`), so the implicit unwrap IS the one point — but so do
comparisons, `??`'s subject, `match`, and the null-check arms, which must NOT refuse.
Cheapest probe: count `convert` callers that are NOT stores (grep `self.convert(` — expect
dozens) and classify by a `first_pass`-style flag.  If a "this is a store" bit can be threaded
(`convert_store` vs `convert`), A is the chokepoint and sites #1–#5 become its callers.

**B. The `⇐` push sites** (B6t: nine of them — block tail/return, call arg ×3, field/param
default, parameter default, struct-literal field, vector element, tuple member, nested
tuple-place RHS) each call ONE new `nstore_check(value_tp, target_tp, what)` before their
`convert`.  Nine callers, one predicate — no threading through `convert`, but a tenth push
site added later is a silent hole (B6t's own finding: `Type::Tuple` was in none of the lists).

Falsifier for either: the Stage A matrix — position (local · field · element · tuple member ·
argument · return · capture · literal) × type kind (integer · narrow · float · boolean ·
character · text · reference · inline struct · vector · keyed) × discharge (none · `?? d` ·
`x?` · `match null` · `== null` · store into non-null) — an `@EXPECT_WARNING`/`@EXPECT_ERROR`
cell wherever undischarged, a silent cell wherever discharged; `make falsify` against the
current build names the cells that pass silently today.  `matrix_axes.py` on the guard.

## Probe result on A (2026-09-05): `convert` has 61 callers in 9 files

`grep -rn "self.convert(" src/parser/*.rs` → 61, and by enclosing function only ~6 are the
assignment seam (`parse_assign_op_inner`); the rest are `parse_key`, the two
`build_null_coalesce_*` (the `??` DEFAULT, which must NOT refuse), `parse_operators` /
`boolean_operator` (comparisons — `x == null` is how nullability is TESTED), `un_ref`,
`process_call_args`, `parse_return`, `parse_field_default`, the vector/trie/spatial index
forms, `rewrite_generic_type_defaults`, …  So threading a "this is a store" bit through
`convert` is 61 per-caller re-assertions with silent omission — A is a spray wearing the
word chokepoint, exactly the shape the protocol's step 2 names.

**Candidate C — make omission LOUD (step 2's second cure).**  Split the junction by NAME:
`convert_store(code, is, should)` refuses `τ? ⤳ τ` per `(N-Store)` (warn/error via
`nstore_softens` + `nstore_narrow`) and `convert(…)` keeps the comparison/coalesce face that
admits the unwrap — and the old name is retired so every one of the 61 callers has to choose,
once, in a reviewable table (caller → store / not).  A new caller cannot forget: the omission
is a compile error, not a silent hole.  N collapses to 1 for the RULE (one function refuses)
and the classification is the design work.  The declared-local ERROR (#4,
`change_var_type`) then becomes the same warning, which is the fold that settles the severity
inconsistency — a behavioural WIDENING (more programs accepted), COMPATIBILITY-permitted.

First claim to probe for C: does today's DN3 branch (site #1) sit INSIDE `convert` or beside
it at the assignment seam?  If beside, the junction the rule wants is not where the teeth are,
and `convert_store` is where they move to.

**Probed (2026-09-05): the teeth are BESIDE the junction.**  The DN3 branch is
`Parser::n_store_violation_inner` (mod.rs:4127), reached from its wrapper at :4124, a
tuple-member loop at :4167 and `vectors.rs:4208` — never from `convert`.  So today the
junction (`convert`) admits `τ? ⤳ τ` and each store site separately remembers to ask
`n_store_violation` first; a site that does not ask is silent.  That is the shape the rule
forbids and the reason candidate C moves the ask INTO a store-flavoured junction
(`convert_store`) rather than adding a sixth caller.

**The per-site census, in full (2026-09-05).**  `n_store_violation` (the wrapper) is asked from
NINE sites — `expressions.rs` ×3 ("the appended value", "the assignment target", "the tuple
element"), `control.rs` ×3 ("the return value" at :1317/:1896/:13568), `operators.rs:2331`
("the return value"), `mod.rs:11793` (the call argument), `objects.rs:4886` ("the field") —
and `n_store_violation_inner` directly from the tuple-member loop (mod.rs:4167) and
`vectors.rs:4208`.  Eleven asks, each naming its `what` by hand, and a twelfth refusal
(`change_var_type`, the declared local) that is not an ask at all and errors where these warn.
So N = 12 today, omission silent.  Candidate C's target: N = 1 (the store face of the
junction), with the eleven `what` strings becoming the store face's one parameter.

## The Stage A matrix, measured today (2026-09-05, joined tree cc65da97, interpreter)

102 cells: position {local, field, element, tuple member, argument, return} × kind {integer,
narrow `u8`, float, text, reference, vector} × discharge {none, `?? d`, `x?`}; a runtime index
`v[i]` is the `τ?` source so no in-bounds proof elides it, and every slot is READ back so no
lint rides along.  Classified by N-Store's own three spellings ("is stored into", "cannot be
stored into", "cannot change type from"); the first cut of this table was built on a
classifier that counted lint noise and was wrong — this one is from `stageA_today.txt`.

**Every discharged cell is silent** (66 of 66).  The undischarged column:

| position | full-width kinds | narrow (`u8`) | vector |
|---|---|---|---|
| local | **ERROR** (#4 `change_var_type`; the rule says WARNING) | ERROR | ERROR |
| field | warn | ERROR | **SILENT — a hole** |
| element | warn | **warn** (the rule says ERROR) | warn |
| tuple member | **SILENT — a hole, every kind** | **SILENT** | **SILENT** |
| argument | warn | ERROR | warn |
| return | warn | ERROR | warn |

Counts: 73 silent (66 discharged + **7 holes**), 20 warn, 9 ERROR.

Three findings the table makes exact: (1) the declared-local site errors at every width — the
one place the code is STRICTER than `(N-Store)`; (2) the narrow escalation is missing at the
element position, where a `u8?` into a `u8` element only warns; (3) SEVEN holes: a `τ?` into a
non-null member of a tuple LITERAL (`t: (integer, integer) = (v[i], 1)`) is not reported for
any kind — the `n_store_violation` ask at `expressions.rs:5745` covers a tuple-PLACE assignment
(`t.0 = …`) and the loop at `mod.rs:4167` a whole tuple into a tuple slot, but the literal's
member-by-member `⇐` (B6t's `seeds_tuple_member_hint` push site) asks nothing — and a
`vector<τ>?` into a non-null vector FIELD of a struct literal is silent while the same value
into an element, an argument or a return warns.  `make falsify` against the current build names
exactly these as the cells that pass silently today.  Files: `stageA/*.loft`, verdicts
`stageA_today.txt`, classifier in the session record.

## Design question 2 — the heap half

`heap_nstore_enabled` (keys.rs:1030): a bare `null` into a non-null reference / collection /
struct-enum at four positions (field, return, element, argument) WARNS and never escalates;
heap LOCALS are refused by `change_var` with its own message (#4 again).  So the heap half is
already routed through branch #2 with `heap_target ||`.  One check, two emitters (scalar
sentinel vs `nullref`) — the emission differs, the CHECK does not; the fold keeps it one check.

## What phase 2 hands over

`keys::nstore_softens(narrow)` (the flag half) and `Parser::nstore_narrow(target, never_error)`
(the width half) are the two facts every one of the five homes will read after the fold.

## Recorded 2026-09-05 (after the `main` measurement)

- The SEVEN silent Stage A holes reproduce on `main` (f4f10cc5, fresh build, both backends:
  the tuple/ref/vector-field cells print no diagnostic while element/local/arg report) — so
  they predate the branch and are filed as **loft#1366** (sev:medium area:parser silent-wrong
  hit-by:loft wa:clean; `Found-via: @PLN153 phase 3`).  Phase 3's commit carries
  `Fixes #1366` + `Contract: settled — (N-Store) already names the slot kinds`.
- loft2's F3 cells (a local assigned from BOTH spellings of `S?`: `x = y; x = o.opt` and the
  reverse; c3 = one spelling, the control) reproduce on `main` too: c1 is a UAF under
  `LOFT_STRICT_STORES=1 LOFT_POISON=1` (the owner witness frees `y`'s store; the caller's `y.n`
  reads garbage), silent without.  The bind `x = o.opt` IS this phase's junction (a tagged
  projection stored into a pointer-spelled local goes through `emit_nullable_slot_read` at the
  bind — L-Null-Which picks the pointer spelling for a local), so it is phase 3's SECOND matrix:
  position × spelling (assign-then-assign both orders, the `if`/`else` form, a parameter vs a
  local as the pointer side; c3 as the control).  Issue: **loft#1367** (filed by loft2, sev:high area:store-lifetime silent-wrong); the fix
  lands here.  Probe: `tmp/pln153/peer_f3/two_spellings.loft` (+ `_main.sh`).

### Stage B cells measured on `main` (2026-09-05, `peer_f3/*.loft`, strict stores + poison, c=false)
| cell | body | result |
|---|---|---|
| c1 | `x = y; x = o.opt` | UAF (y's store freed by the owner witness); caller's `y.n` garbage |
| b1 | `if c { x = y } else { x = o.opt }` | CORRECT (99 / o.opt.n=99 / y.n=3) |
| b2 | `if c { x = o.opt } else { x = y }` | SILENT-WRONG: 99 / o.opt.n=2 / y.n=3 — the write landed nowhere |
| w1 | `x: S? = y; x = o.opt` | UAF (the declared pointer spelling is NOT a workaround) |
| w2 | `x: S? = o.opt; x = y` | SILENT-WRONG (99 / 2 / 3) |
| d1 | `x = o.opt ?? y` | REFUSED, leaking `__nullable<S>` into the message ("cannot change type from S? to __nullable<S>?", "Unknown field __nullable<S>.n") |
| p1 | `x = y` | CORRECT (control) |
Reading: the LAST-parsed assignment's spelling wins the local's type; when the pointer wins
(b2, w2, c2) the projection is bound UNCONVERTED and `x.n` writes the tag; when the tagged
spelling wins (c1, w1) the pointer parameter is read as a tagged record and the witness frees
it.  b1 is correct because the `if` arm's type joins in the other order.  Cure at the bind
(`emit_nullable_slot_read` for a tagged projection into a local) cures all seven.

## The design, decided from the census (2026-09-05)

**Census instrument.** `#[track_caller]` on `convert` + an env-gated trace in its
Optional-SOURCE arm, run over the corpus (`unwrap_census.sh`, scratch worktree `wt-p3`):
6014 `τ? ⤳ τ` peels in 1268 files.  Every one of the eleven site-level asks is followed by a
peel through that ONE arm (asked=true at mod.rs:11979 ×3135, vectors.rs ×64, control.rs ×18,
expressions.rs ×21, objects.rs ×4, fields.rs ×4, operators.rs ×12).  The peels that reach the
arm WITHOUT an ask are: the call-argument site when the ask is gated off (2650 — comparisons
and null-transparent callees, `pln102-null-comparison-uniform` ×92, `292-pln17-three-state`
×106; the stdlib itself contributes 1), the null TESTS (`operators.rs:685`, `float? -> boolean`
×16), the CONDITION (`convert_condition`, 3), one vector-literal boolean (1) — and the INDEX
(`fields.rs:2051`, `integer? -> integer` ×6 in 6 corpus files: `s[pos..]` after a `find`).
The first four are the ADMITTING faces the rule names (a test of nullability is not a store);
the index is a slot and is silent today: a HOLE, the eighth.

**Invariant, restated where it lives.**  `convert`'s Optional-source arm IS the junction every
`τ? ⤳ τ` passes (the census is the proof: no peel happens anywhere else).  So the arm asks
`(N-Store)`'s τ? half there, unless the caller ADMITS the unwrap explicitly:

- `convert_store(code, is, should, what, at)` — the STORE face: pushes `(what, at)` on a
  context stack, converts, pops.  The arm reads the top for its wording; with no context it
  still asks, worded generically ("a non-null slot") — a site that forgot degrades the
  MESSAGE, never the rule.  The element-wise tuple arm prefixes `element i of ` as it
  recurses, which is how the seven tuple-literal holes get their proper wording for free.
- `convert_admitting(code, is, should)` — the TEST face: a depth counter the arm honours.
  `convert_condition`, the null tests (`operators.rs:685`), the call-argument site when its
  ask is gated off (overload trial / null-transparent callee), and nothing else.  A caller
  that should admit and does not gets a SPURIOUS WARNING — visible, the corpus shows it.
- The DN1 (bare `null`) half and the tuple recursion stay in `n_store_violation` at the
  sites: a bare-null source does not always reach `convert` (`if t == Type::Null {
  null_value }` bypasses it at the return sites), so its home is unchanged by this phase
  and named as the second face (loft#1313's heap gate is its chokepoint).
- A lowering that bypasses `convert` altogether (the struct literal's vector-field deep copy,
  `OpAppendVector` at objects.rs ~4707) asks for itself — the genuinely separate push sites,
  each named; that is the `field/vector` hole's fix.

**Failure paths written down before building.**
1. Double report: a site asks (DN3) AND the arm asks → the site's DN3 half is removed; only
   DN1 stays at sites.  Guard: every REFUSE cell of Stage A expects exactly one diagnostic.
2. A refusal (narrow ERROR) inside the arm must not be followed by the caller's generic
   "cannot assign" — the arm returns `true` (accepted-with-diagnostic) after an ERROR.
3. Stale context: the stack is pushed/popped around one `convert`; nested literal converts
   push their own; the tuple arm's prefix is pushed/popped per element.
4. The admitting counter leaking: `convert_admitting` restores it on every path (a guard
   struct or an explicit decrement after the call; no early return between).
5. Over-reach: a peel that is NEITHER a store nor a listed admitting face — the corpus run
   after the change lists every new diagnostic; each is a hole (keep) or an admitting face
   missed (add).  Both outcomes are visible; silence is impossible by construction.
6. The index cell: `s[i]` / `s[a..b]` with a nullable bound now warns "the index" — six
   corpus files; each is read to confirm the warning is right there (a `find` that may miss).

**Split into three commits, each red on its own:** 3a the arm + faces + the eight holes
(`Fixes #1366`); 3b the declared-local severity fold (`change_var_type` ERROR → the split's
WARNING; 9 corpus files re-pinned; `types.md:159` corrected; `Contract: strained`); 3c the
bind of a tagged projection into a pointer-spelled local (`Fixes #1367`).

## Phase 3a built and measured (2026-09-05, scratch worktree `wt-p3`, transplant pending)
- Stage A: the 7 holes → warn, 0 other cells moved; admitting probe (16 reads) silent on both
  builds, values identical; the if-arm join needed `convert_admitting` (it warned once —
  `fn maybe() -> integer? { if k > 5 { 7 } else { null } }` — the one over-reach the corpus
  caught); the Optional-TARGET null arm's recursion needed admitting too (`File { ref: null }`
  on an `i32?` field read as a narrow refusal and the STDLIB refused to load).
- Guards written: 1366-tuple-member, 1366-vector-field, 153-nullable-index (0 warnings on
  806a8d84, 6/1/2 on 3a), 153-n-store-admitting-faces-hold.
- types.md (N-Store) names its slots (index included) and its non-stores.

## Phase 3b built and measured (2026-09-05, in `~/workspace/loft-1361` on 8498fdf1)
- The cell list is `~/workspace/pln153-scratch/stage3b/CELLS.md` (25 cells, written first);
  `run.sh <loft>` runs them.  Every declared cell reports ONCE and holds the null; every
  inferred cell widens silently; `?? d` / `x?` / `j: integer?` stay silent.
- Two more sites than the design named: the tuple-DESTRUCTURE store (`(a, b) = (v[i], 1)`
  with `a` declared) and the write-back `&τ` PARAMETER (silent before — the `RefVar` peel).
- The hidden-buffer refinement (`Parser::author_declared`): the text-return hoist promotes a
  local to a hidden `&text` argument under the author's name; `argument || annotated` alone
  called five corpus files' `got = maybe(i)` a parameter store.  Read off the definition's
  `hidden` attribute, not declared per hoist site.
- Stage A: 5 cells moved (local × integer/float/text/ref/vector, none: error → warn).
- loft#1369 filed (native leak, pre-existing on the declared spelling).
