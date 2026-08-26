<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/IMPLEMENTATIONS.md — one rule, how many implementations?

A rule in [formal/](README.md) is usually enforced by a **membership test over `Type`
variants**: *is this a scalar*, *is this a keyed collection*, *does this own a store*. Written
inline at each site, the copies **drift** — and a drifted copy is not a tidiness problem, it is
a defect: loft#1006 was two spellings of one tuple-element list disagreeing, and loft#1065 was a
stale mirror of `coalesce_not_null`.

This file is the **checklist**: for each rule that looks like it can have more than one
implementation, where the implementations are and what the verdict is.

**Regenerate the raw data** — `python3 scripts/rule_predicate_audit.py` (add `--near` for the
lists that already differ by exactly one variant). It is a REPORT, never a gate: some repeats
are genuinely different questions that share a list *today*, and merging those would couple two
rules that must stay free to diverge. The verdict column is the judgement the script cannot make.

> **⚠ A shared list is not automatically one rule.** Before merging, ask what each site is
> asking. Two sites that agree today because the language is small will silently constrain each
> other later. `Const-ScalarCollapse` and "which elements may a `&(…)` hold" happen to be the
> same set of types — they are not the same question, and the merge below only holds because
> both derive from ONE deeper fact (the layout), which is what the shared home names.

## Rule tags — `@FR-<Rule>`, and what the count cost to get right

The checklist below rests on being able to ask *which sites enforce this rule?* and get an exact
answer. `scripts/rule_tags.py` is that tool — `list` · `check` · `sites <tag>` · `dups` — and
`check` is a gate: every citation resolves, no rule is defined twice.

| | |
|---|---|
| defined rules (fenced `(Name)` blocks + deviation entries) | **285** |
| prefix pairs (`@FR-B-View` ⊂ `@FR-B-View-Base`) | **21** |
| family prefixes used in prose, NOT rules | `B-Ref`, `D-op`, `D-own`, `D-cap`, `D-op-null` |
| cited from code today | 5 rules, 7 sites |

**Two things the check found before a single citation was written, and neither was reachable by
reading:**

- **`L-Ref` was two different rules.** `closures.md` defined it as *a bare function name is a
  fn-ref value*, `layout.md` as *a stored reference / collection field* — both docs use `L-` as
  their prefix, one for Lambda and one for Layout. Renamed to `L-FnRef` in `closures.md` (the
  side with one reference, and the more accurate name).
- **`D-bind-11`'s entry had been silently DELETED.** Its register line still read `OPEN: 1
  (D-bind-11)` while the entry itself was gone — removed on 2026-08-23 by an edit whose slice
  anchor sat inside its body while rewriting D-bind-12. The doc still read plausibly; nothing
  else would have caught it. It surfaced as an unresolvable `@FR-D-bind-11` citation, and the
  entry is restored.

⚠ **The count moved five times, and every move was the instrument learning what it was
counting** — 361 → 356 → 251 → 268 → 285, and collisions 0 → 33 → 23 → 2 → 1 → 0. Each step was
a false positive with a diagnosis, not a guess: a regex that could not span a second hyphen
reported 0 collisions; markdown section headers (`## Rules`, `## Notation`) counted as rule
definitions; a parenthesised MENTION in prose counted as a definition; deviations turned out to
have TWO spellings in use (`### D-own-7 —` and `> **D-bind-11 —`) so only half were registered;
and a blockquote cross-reference (`> **D-bind-10**:`) read as a second definition. **A number
from an instrument that cannot yet represent what it counts is not a measurement** — the same
lesson this file keeps producing, here applied to the file itself.

The convention is written up in [README.md § Rule tags](README.md) and
[CLAUDE.md § Tracker tags](../../../CLAUDE.md). Until citations are widespread the site counts in
the checklist below are lower bounds, and `scripts/rule_predicate_audit.py`'s shape-matching is
the complementary instrument — it finds duplicates that share code, `rule_tags.py dups` finds
those that share a RULE.

## Already merged — do not redo

| rule / question | the ONE home | why it was merged |
|---|---|---|
| which elements a `&(…)` admits | `data::ref_tuple_element_ok` → `data::is_scalar` | loft#1006 was the signature guard and the two `RefTuple` codegen arms disagreeing; 2026-08-24 it turned out to disagree with `generation`'s own `is_scalar` too |
| is this a SCALAR | `data::is_scalar` | three copies, one of which (`ref_tuple_element_ok`) had drifted — see checklist #1 |
| does a tuple carry a fn-ref (at any depth) | `data::tuple_carries_fn_ref` | three sites read it (interp push, native slot hand-down, the reachability walk); each had its own shallow reading |
| what an ABSENT / narrow field holds | `Stores::write_absent_value`, `write_narrow_value`, `narrow_is_null` | two JSON walkers answered "what does an absent field hold?" per type instead of asking the declaration (`layout.md` L-Null) |
| a `&` in source becoming `Type::RefVar` | `Parser::ref_var_type` | the parameter, the annotated local and the inferred `b = &a` asked three different lists (D-tup-2) |
| may a move-elide retarget across this statement | `scopes::collect_move_disturbed` | the Record and Construct shapes had the identical hole; now one predicate, parameterised by copy-op + source-arg |
| which `@EXPECT_*` tag a corpus file carries | `common::expect_tag` | the interpreter and native runners skipped on different readings, costing 79 assertions |
| is this a cell / primitive-vector element target | `cell_struct_name`, `is_primitive_vector_element_target` | audited 2026-08-22; the three copies of each list agree |

## The checklist — candidates, with site counts measured 2026-08-24

Status: ☐ to assess · ⚠ drift found · ✅ merged · ⛔ deliberately separate.

| # | the question | rule(s) it enforces | sites | status |
|---|---|---|---|---|
| 1 | **is this a scalar** (`Integer\|Float\|Single\|Character\|Boolean\|Enum(_,false,_)`) | `types.md` scalar/heap split; the `&(…)` admitted-element rule | 8 → **3 merged**, 5 left | ✅ **merged + drift FIXED.** `generation/`'s two copies included `Enum(_, false, _)`; `ref_tuple_element_ok` did not — so `&(Col, Col)` was refused while `&(boolean, boolean)` was admitted, on an identical 1-byte layout. One home: `data::is_scalar`. The 5 remaining sites (`scopes.rs:3898`, `generation/emit.rs`, `parser/operators.rs:47` = `Const-ScalarCollapse`, `parser/mod.rs:4932`, +1) spell the BARE five, so adopting them ADDS value enums at each — a behaviour change per site needing its own probe. Left open deliberately. |
| 2 | **is this carried as a DbRef** (`Reference\|Vector\|Enum(_,true,_)`) | `@FR-Col-Store` — home is `data::is_dbref` | 43 → **3 fixed**, 40 to read | ⚠ **the short list is a BUG source** — see § The DbRef set below. Home exists now; the remaining 41 each need reading, since some may legitimately want only three |
| 3 | **is this a KEYED collection** (`Hash\|Index\|Radix\|Sorted\|Trie`) | `@FR-Col-Hash` · `-Sorted` · `-Index` · `-Spatial` · `-Trie` | 16 → **1** | ✅ **merged onto `vectors::is_keyed`** — see § The keyed collections below |
| 4 | **is this a collection** (keyed `+ Vector`) | `@FR-Col-Store` — home is `vectors::is_collection` | 13 → **1** | ✅ **merged**, IR byte-identical on 854/854. THREE homes existed, not one — see below |
| 5 | **is this a DbRef-represented type** | `@FR-Col-Store` — home is `data::is_dbref` | 11 → **1** | ✅ **merged**, IR byte-identical on 854/854. And it found a duplicate home I had just created myself — see below |
| 6 | **the narrow integer widths** (`Byte\|Int\|Short\|ShortRaw`) | `@FR-L-Narrow` (width set) · `@FR-L-Null` (the encoding) | 6 → **5 cited, 1 excluded** | ✅ **evaluated — and only 3 of them are the same question.** See § The narrow widths below. |
| 7 | **the value-carrying `Value` wrappers** | `@FR-F-Drop` / `@FR-F-Block` | 32 + 13 + 9 + 3 + 2 | ✅ **evaluated — FOUR questions, not one.** Not mergeable; one real gap fixed. See below |
| 8 | **which `Value` shapes hold a statement list** (`Insert\|Parallel\|Tuple`) | `operational.md` | **8** | ☐ a `Value` list rather than a `Type` list; the audit script only reads `Type::` today |

## The narrow widths, evaluated with the tags (checklist #6, 2026-08-24)

The first family worked through with `@FR-` citations rather than shape-matching, and the
result is the case that makes the exercise worth doing: **a shared type list, three different
questions.**

| sites | question | verdict |
|---|---|---|
| `native.rs` ×2, `database/structures.rs` | dispatch on width, then delegate to `write_narrow_value` | **one rule, already merged** — the shared home carries `@FR-L-Null`; these are its dispatch guards |
| `state/io.rs` ×2 | same width classification, **raw i64 in a variable slot** | **must NOT be folded** |
| `database/allocation.rs::is_inline_scalar` | inline-vs-relocated for the paged store | **not this family** — its list is a superset (`+ Parts::Base`) asking a different question |

**The `io.rs` pair is the interesting one, and the code already said so.** Its own comment
reads *"stored raw as i64 (OpPutInt), **not** via the +1-encoded `Parts::Byte/Short` encoding
that structs use (nor the `i32::MIN` null sentinel of `Parts::Int`)"* — the same four types,
the deliberately opposite encoding. A shape-matcher ranks it identical to the three that DO
merge; only reading what each site asks separates them. Both now carry `@FR-L-Narrow` for the
width classification plus an explicit note that they are not fold candidates, so the next
reader does not have to re-derive it.

⚠ **A rules GAP this surfaced, recorded rather than filled.** `@FR-L-Narrow` states the STORED
width (`u8 → 1 B, u16/i16 → 2 B, i32 → 4 B, else 8 B`) and `@FR-L-Null` the field encoding.
Neither says that a narrow value in a VARIABLE slot is a raw `i64` — the very fact the `io.rs`
pair depends on and comments at length. The rules cannot currently express the distinction the
code is built on, which per [README](README.md) means the RULE wants extending; minting one is
a spec decision and is not taken here.

**What the citations buy immediately:** `idx tag:@FR-L-Narrow` (or
`scripts/rule_tags.py sites L-Narrow`) lists every site that must be revisited if a width is
ever added to the set — which was previously a grep for four `Parts::` names that also returned
`is_inline_scalar`, a site that must NOT change with them.

⚠ The audit script needed widening to see this family at all: it read `Type::` only, and the
narrow widths live in `Parts::` — the LAYOUT view. An instrument that cannot represent half the
vocabulary reports a clean sweep of the half it can see.

## The keyed collections, merged (checklist #3, 2026-08-24)

The clearest instance so far of *structure that already exists, with several implementations
inside it*. The predicate **"is this a keyed collection"** was spelled **16 times**:

- `vectors::is_keyed` — `pub(crate)`, documented, reachable from everywhere;
- `objects::is_keyed_collection` — a PRIVATE second helper holding the same five variants in a
  different order;
- **14 inline `matches!` copies** across `collections.rs`, `expressions.rs` (×5), `mod.rs` (×2),
  `operators.rs`, `scopes.rs` (×3) and `state/codegen.rs` (×2).

Adding a sixth keyed kind meant finding sixteen places, and nothing connected them. All now
call `is_keyed`; `is_keyed_collection` delegates to it rather than restating it.

**Behaviour-preserving, proven rather than asserted:** emitted IR is byte-identical on
**853 of 853** `tests/scripts`. The predicate was already identical everywhere — which is why
this one is a pure merge and not a bug hunt, and it is the honest opposite outcome to the
narrow widths, where three sites that looked the same were three different questions.

⚠ **A rules gap, recorded not filled — and larger than the narrow-slot one.**
`Col-Hash` / `Col-Sorted` / `Col-Index` / `Col-Spatial` / `Col-Trie` each define ONE kind. **No
rule names the KEYED FAMILY as a category**, yet that category is exactly what sixteen sites
were testing. `is_keyed` therefore cites all five, which is accurate rather than tidy: a sixth
keyed kind must update it, so `idx tag:@FR-Col-Spatial` has to return it.

Checklist #4 (`is_collection`) resolves in the same breath: its home exists and is literally
`is_keyed(tp) || Vector`, so #3 and #4 differ by that one variant **by design**. The near-dup
detector flagged them as a candidate drift pair; reading them shows a deliberate pair. That is
the detector working as intended — it proposes, the reading disposes.

## The DbRef set — a list that drifts SHORT, and the bug it produced (checklist #2, 2026-08-24)

`Reference | Vector | Enum(_, true, _)` appears at **43 sites**. It looks like "the heap
types" and it is not: `element_stack_size` gives **eight** types a `DbRef` — those three plus
`Sorted`, `Index`, `Hash`, `Radix`, `Trie`. The five keyed collections are the ones that get
forgotten, because they are reached by key and do not read as references at the call site.

**It is not a tidiness problem. It produced a live bug in the one place I probed.**
`generation/coroutine.rs` chose the generator's yield channel with the short list, so a
generator over a keyed collection sent a handle down the `next_i64` channel:

```loft
fn g() -> iterator<hash<E[k]>> { a: hash<E[k]> = [E { k: 1, v: "a" }]; yield a; }
```

`--native` refused to compile (a raw rustc `E0605` handed to the user); `--interpret` reported
`BUG (#306): a stack-record ref was treated as an owned heap store`. The same generator over a
`vector` was fine — the boundary is exactly the list. Both sites now read `data::is_dbref`,
and `--native` answers correctly.

**Why it survived: the corpus has never yielded a DbRef.** `tests/scripts` covers
`iterator<integer>` (45 files), `<text>` (10), `<single>` and `<float>` — and nothing carried
by handle. `coroutine-yields-a-dbref-value.loft` is the first coverage of that channel
(vector, struct reference, and the scalar channel beside them as the control).

**The interpreter half was a SECOND site with the same short list — now fixed.**
`collections.rs` binds the consumer's loop variable as a BORROW of the generator
(`x: ref(E)["___gen_1"]`) precisely so the per-iteration free is never emitted, and its comment
already says what happens otherwise: *"whole-store-freed the generator's state store … and
tripped the #306 stack-store guard"*. That arm listed `Reference | Enum(_, true, _) | Vector`.
A keyed yield therefore bound `x` with NO dep, the scope machinery read it as an OWNER, and the
free that arm exists to prevent produced exactly the `BUG (#306)` its own comment describes.
Both sites now read `data::is_dbref`; all five DbRef yield shapes answer correctly on both
backends with no leak.

⚠ `coroutine_layout::next_operands` is still short — six of the eight, no `Radix`/`Trie`.
Probed: a `spatial` yield is correct anyway, so it does not bite. The guard now carries a
spatial cell to keep it that way rather than leaving the discrepancy unattended.

**The other 40 were cleared by a SENTINEL, not by reading them.** Reading forty sites is the
kind of task that produces a confident wrong answer; the cheap instrument is better. Each
short-list site was temporarily rewritten as a probe returning the same answer while reporting
when the FULL `is_dbref` set would have said yes — a divergence is exactly *"a keyed collection
reached a site that does not list it"*. Over `tests/scripts` + `tests/docs`:

| | |
|---|---|
| sites instrumented | **38** (2 sit nested inside another `matches!` and were read by hand) |
| sites that EVER diverge | **4** |
| sites that never see a keyed collection | **34** |

The four are all return-buffer / owned-ref FREE decisions — `scopes.rs:4390` (the
`OpFreeRefIfDistinct` witness pair, 492 divergences), `state/codegen.rs:2064` (the loft#615
`??`-subject free, 136), and the two return-buffer promotion gates (`parser/mod.rs:2223`,
`parser/definitions.rs:1601`). Each was then probed against **its own comment's documented
failure mode**, including under `LOFT_POISON=1` — which those comments name as the instrument
for the silent case: a keyed `??` subject in a loop, and a keyed return-buffer adopted in a
loop. Both answer correctly, with no leak. A keyed return simply does not get a buffer, and the
path it takes instead is right.

So the short list is **not** wrong at the remaining 40: three sites were, and they are fixed.
The sentinel is what makes that a measurement rather than an opinion, and it cost a fraction of
what reading forty sites would have.

## Checklist #5, and the duplicate I created while writing the checklist (2026-08-24)

Ten sites spelled the FULL eight-variant DbRef list inline; all ten now call
`data::is_dbref`. Emitted IR is **byte-identical on 854 of 854** `tests/scripts`, so this one
really was the pure conversion #3 was.

⚠ **The tenth site was `Parser::is_heap_handle` — an existing home for exactly this
predicate**, carrying the doc *"One home, because the answer is shared by the `??` null check,
the `if`/`while`/`assert` condition, and the `!x` null test."*

`data::is_dbref` was added earlier the same day, for the coroutine fix. It is a **second home
for a predicate that already had one**, created by me, in the middle of the work whose entire
subject is that duplicates get written because nobody looks. The reason is exact and worth
keeping: I searched for a home for the SHORT three-variant list, found none, and stopped. I
never searched for a home for the FULL list — the one I was about to write.

That is the failure mode this file exists to catch, and it caught it, but only on the NEXT
family. What would have caught it at the time is asking the question about **the predicate I
was creating**, not about the list I was replacing.

`is_heap_handle` now delegates to `is_dbref`, keeping its name and the `.base()` peel its
null-check callers rely on; both ends carry a note saying which came first and why the search
missed it.

## Checklist #4 — three homes for one predicate (2026-08-24)

"Is this a collection" had **three** named homes and ten inline copies:

- `vectors::is_collection` — `pub(crate)`, and the only one that DERIVES the answer
  (`is_keyed(tp) || Vector`) instead of restating the six variants;
- `generation::is_collection_field` — restated;
- `objects::is_collection_type` — restated.

All three held the identical set. The two restaters now delegate and the ten inline copies
call the derived one, so adding a collection kind touches `is_keyed` and nothing else. IR is
**byte-identical on 854 of 854** `tests/scripts`.

**The pattern across the four merged families is worth stating.** #3 had two homes + 14 inline,
#5 had two homes + 9, #4 had three homes + 10. In every case a home already existed — often
documented as *"one home"* — and the copies accumulated beside it anyway. The duplication is
not a failure to CREATE the abstraction; it is a failure to FIND one that is already there,
which is why the instrument matters more than the discipline.

⚠ Derivation beats restatement, and this family shows why: `vectors::is_collection` is the
only one of the three that cannot drift from `is_keyed`, because it is defined in terms of it.
The other two would have needed updating by hand had a keyed kind been added — the exact
maintenance the merge removes.

## Checklist #7 — four questions wearing one name (2026-08-24)

`Return` appears in five near-variant arm sets, and the temptation is to read them as one
"terminator" question spelled five ways. They are not. Naming them is the deliverable, because
merging them would be the too-early-abstraction failure with real consequences:

| set | sites | the question | verdict |
|---|---|---|---|
| `Drop \| Return` | **32** | the **function-body TAIL** wrapper (`result_var`, `tail_if_has_null_arm`, `push_text_arms_into`) | ⛔ separate — a `yield` is not a function tail (the generator RESUMES after it) and a `break with` is a LOOP tail |
| `BreakWith \| Drop \| Return \| Yield` | 13 | a node that **wraps an inner value** — the generic walker shape | the complete set |
| `Drop \| Return \| Yield` | 9 | the same walker question, three of the four | 7 of 9 carry a SEPARATE `BreakWith` arm (it takes a label as well as a value, so it cannot share one); 2 did not |
| `BreakWith \| Return` · `Break \| BreakWith \| Continue \| Return` | 3 · 2 | **loop exit** | ⛔ separate again |

**The one gap — and the claim I made about it was wrong.** `scopes::walk_check`, the
`check_arg_ref_allocs` validator that reports a `Set(v, Null)` nested inside a call argument
("corrupts the CallRef arg layout", A5.6), walked `Return`/`Drop`/`Yield` and not `BreakWith`.
I recorded that "a violation under a `break with f(…)` was therefore never reported". There is
no such program: `break with` is not loft syntax, and `Value::BreakWith` has **no producer at
all** (#8 below). The arm stays — it is free and it makes the walker match the keystone — but it
fixed nothing, because nothing could reach it. The lesson is the one this checklist keeps
re-teaching: an arm-set gap is a hypothesis about REACHABILITY, and the variant list does not
establish that anything reaches it.

**The other gap is deliberate and now says so.** `scopes::prepend_to_scope` omits `BreakWith`
too, but that relocation is BEST-EFFORT with a correct fallback — leaving the null-init at body
position 0 — which is exactly what the @PLN57 cluster-I false alarm established when the same
walker could not reach a `map`/`filter`/lambda body either. Adding the arm would relocate in one
more shape; it would not fix anything. The comment now records that so the next reader does not
"complete" it.

⚠ **These are match ARMS, not predicates, and that changes the tool.** The merged families (#1,
#3, #4, #5) were membership tests that collapse to a shared function. An arm set has a *body*
per site, so there is nothing to call — the reusable artefact here is the TABLE above, not a
helper. Recognising which kind you have is what stops a merge being attempted where it cannot
land.

## Checklist #8 — the keystone that says every walker derives from it (2026-08-24)

The filed question was "which `Value` shapes hold a statement list", expecting a merge like #3/#4.
It is not a merge, and the two arm-sets it started from (`Insert | Parallel | Tuple`, 8 sites, and
`Call | Insert | Parallel | Tuple`, 5) differ only by whether `Call` can share the arm body — it
carries a `def_nr` the others do not. Both are ordered child lists; all 13 descend into calls.
(My first pass reported `parser/operators.rs:4052` as having no `Call` arm nearby. It has two, at
3929 and 3954 — outside the ±55-line window I measured with. Fourth window artifact of this
sweep; the window is now part of what gets stated with the number.)

**The real finding is one level up.** `Value::for_each_child` (data.rs) already IS the merged home,
and its doc says so: *"the ONE place that knows `Value`'s tree shape … Every traversal derives from
this — the match is exhaustive on purpose, so a new `Value` variant forces a decision here and
every walker inherits the edge."* That is a claim, and claims get re-measured:

| how a recursive multi-variant walker descends | count | what a new edge does |
|---|---|---|
| via the keystone | 31 | inherited — nothing to do |
| own match, exhaustive (no catch-all) | 22 | breaks the build; the author must decide |
| own match + `_ =>` catch-all | **127** | falls into `_`, **silently** |

`scripts/ir_walker_audit.py walkers` re-measures it. The third row is the hazard, and the
MECHANISM has fired twice — `inline_ref_set_in`'s hand-rolled predecessor *"treated `BreakWith`
as a leaf and missed a `Set` inside its value"* (parser/expressions.rs:121), and
`scopes::walk_check` had the identical hole (#7).

⚠ **But both landed on `BreakWith`, and `BreakWith` turns out to be unreachable — so neither was
a live defect.** They demonstrate the shape, not the damage, and it would be sleight of hand to
count them as evidence that the 127 are costing anything today. What they honestly establish is
narrower and still worth having: a hand-rolled walker DOES silently drop an edge the keystone
would have supplied, and nothing in the build, the tests, or review noticed for years — twice.
**The measurement that would settle it has not been made:** for each of the 127, which of the
edges it omits are REACHABLE? That is a different query from this one, and it is the honest next
step rather than a mass conversion. QUALITY.md § B2.

**Two `Value` variants have no producer.** `BreakWith` and `ParFor` can only be *rebuilt* inside a
walker's own arm, or *deserialized* — and a deserializer returns only what a producer once wrote,
so both are closed cycles with no source. No file under `src/parser/` has ever constructed either,
under any name, in any commit. Measured two independent ways, because neither alone is an oracle:

| variant | nodes in 854-program corpus | construction sites | verdict |
|---|---|---|---|
| `Single` · `Loop` | 147,854 · 22,176 | screen says "no producer" | **live** — built through a helper the screen reads as a rebuild |
| `Parallel` | **4** | same | live, but the corpus barely reaches it |
| `Iter` · `RawExpr` | 0 · 0 | real producer found | live — `Iter` is lowered away before the snapshot, `RawExpr` is built after it |
| **`BreakWith` · `ParFor`** | **0 · 0** | rebuild + serializer only | **dead** |

Corpus-absence alone proves nothing (`Iter`, `RawExpr`); a producer screen alone proves nothing
(`Loop`, `Single`, `Parallel`). The intersection is the answer: `scripts/ir_walker_audit.py dead`.

**Why this is worth the words.** A producerless variant is not free. It costs every walker an arm;
66 hand-rolled walkers get `BreakWith` wrong; and *no test can reach any of those arms*, so the
defect is structurally invisible — which is exactly how it was found and mis-diagnosed twice. Both
declarations now carry the measurement, so the next reader does not audit 66 arms again.

**Removed (2026-08-24).** The owner's call was to delete both. `Value::BreakWith` and
`Value::ParFor` are gone, with `ParForBody`, their walker arms, both serializer shapes, the
`IrNode` accessors, the store-schema types and the round-trip tests that were their only exercise.
`scripts/ir_walker_audit.py dead` now reports no dead variant at all, which is the check that
closes this.

The compatibility question turned out to be answerable rather than a judgement call: removing
`NdBreakWith` (19) and `NdParFor` (33) renumbers every later discriminant, but no store image
survives the change, because `startup_cache::save_program` writes `cache::build_signature()` as the
manifest's first line so a binary upgrade invalidates the bundle. The numbering closes up, and
`data_store::baked_layout_mirrors_loft_schema` is the gate that proves the baked `DISC_*` / `ND*`
constants still match the regenerated schema — it failed on the first renumber attempt and named
the exact constant, which is what a layout gate is for.

## The variable-lifetime map (2026-08-24)

The `deps` ownership system is the subsystem loft is most often wrong in and hardest to
reason about, so this is the rule → site map for it. Every row is cited in the code, so
`python3 scripts/rule_tags.py sites @FR-O-Deps` (etc.) answers *"what implements this?"*
without anyone having to remember.

| rule | what it says | implemented at |
|---|---|---|
| `@FR-O-Deps` | one fact; every lifetime decision derives from it | `data::Deps` (`src/data.rs`) — the type itself |
| `@FR-O-Borrow` | an aliasing value names its source; borrowers are skip-free | the `Deps` list; `Function::make_independent` strips a dep to promote a borrow to owner |
| `@FR-O-Owner` · `@FR-O-Derived` | single owner; free is DERIVED, once, at scope exit | `Scopes::get_free_vars` (`src/scopes.rs`) — the scope-exit sweep |
| `@FR-O-Move` | a returned store transfers to the caller | `get_free_vars`'s `ret_var` / `return_sources` suppression; `Parser::ref_return` (`src/parser/control.rs`) |
| `@FR-O-Complete` | per binding, per path — set-and-reconcile | `Scopes::scan_if`'s intersect of `owned_refs` across both arms (`src/scopes.rs`) |
| `@FR-O-NoDiverge` | both backends translate the SAME facts | structural: `scopes` decides and writes `OpFreeRef` into the IR; the emitters translate |

**The load-bearing invariant is `O-Complete`, not soundness.** loft has no user-facing
borrow checker — the user writes naively and the compiler must always find a lowering — so
an incomplete fact is not a compile error someone fixes. It is a miscompile or a leak. That
is why the `if` reconcile INTERSECTS the two arms rather than unioning them.

### ⚠ The model has TWO facts, and its rules name one

`O-Deps` says every decision derives from `deps`. The way a site actually reads ownership
off `deps` is `tp.depend().is_empty()` — and the code states plainly what that is worth:

> *"loft#723 — empty deps is only a PROXY for ownership, and it reads 'owned' for a borrow
> whose dep list was never populated."* (`src/state/codegen.rs`)

The repair was a SECOND carried fact, `Function::is_skip_free`, whose contract is "never
emit `OpFreeRef` for this var" and which must VETO the proxy. Consulting it only at the
scope-exit sweep left an unconditional pre-Set free reachable inside a loop body, where it
landed on the *next* iteration's store — stale bytes without `LOFT_POISON`, SIGSEGV with it.

**Measured 2026-08-24:** 24 functions test `depend().is_empty()`; 9 consult `skip_free`,
15 do not. ⚠ **That count does not survive inspection, and how it fails is the real
result.** Checking the largest proxy-without-veto site that frees — `Scopes::scan_set` —
its `OpFreeRef` turns out to be gated on neither: the condition is
`owned_refs.get(&v) == Some(&self.loops.len())`, a THIRD carried fact (the latest
assignment's ownership, tagged with the loop depth at which it was taken). The function
contains the proxy for a different question entirely.

**So the model carries at least three facts where its rules name one:**

| fact | where | what it answers |
|---|---|---|
| `deps` (`tp.depend().is_empty()`) | the type | is this a borrow? — a PROXY, wrong for an unpopulated dep list |
| `Function::is_skip_free` | the variable table | never free this one — the veto the proxy needs (loft#723) |
| `Scopes::owned_refs` | the scan state | did the LATEST assignment leave it owning a store, and at which loop depth |

None of the three is redundant: `owned_refs` carries a temporal fact (*latest* assignment)
and a loop-depth fact that a type-level dep list structurally cannot, which is why the
transition free is gated on it and not on `deps`.

The gap was therefore not "15 sites forgot the veto" — it was that **which fact a lifetime
decision should read was nowhere written down**, so a reader could not tell a site that
legitimately reads one from a site that reached for the wrong one.

✅ **`ownership.md` now names all four** (2026-08-24), and the fourth is the one that
reframes the rest:

| rule | fact | implemented at |
|---|---|---|
| `@FR-O-Oracle` | the own-vs-borrow derivation, from the IR | `use_analysis::ownership_of` |
| `@FR-O-Proxy` | empty `deps` as a stand-in for "owner" — unsound alone | `Type::depend().is_empty()`, 24 sites |
| `@FR-O-Override` | the never-free veto that makes the proxy safe at a free | `Function::is_skip_free` |
| `@FR-O-Latest` | latest assignment's ownership + the LOOP DEPTH it was taken at | `Scopes::owned_refs` |

**`deps` is not the oracle.** `ownership_of` derives own-vs-borrow from the IR — a store
mint is `Owned`, a projection is `Borrowed(base)`, a call resolves through the callee's
return summary — and it **never consults `deps`**. The two are independent derivations of
the same question, which is why they can disagree, and loft#723 is what that looks like.

`O-Proxy` carries the first checkable obligation in this space: *a site that FREES on the
empty-deps proxy must also consult `O-Override`.*

### ✅ That obligation is now enforced (2026-08-24)

`scripts/o_proxy_check.py`, gated by `doc_hygiene::o_proxy_frees_consult_the_override` and
runnable as `make o-proxy-check`. 20 positive proxy sites, 8 negated, **0 violations**.

**Three discriminations, each of which was a false positive first:**

1. **`!tp.depend().is_empty()` is a different question** — "is this a borrow?" — and needs
   no override, since a borrow is not freed either way. 8 of the 28 sites are this form.
2. **The free must be in the region the condition GATES**, not merely nearby. A 20-line
   window bled across a function boundary and accused `dispatch::materialises_element`, a
   classifier that frees nothing.
3. **Comments are not code.** Matching `OpFreeRef` in prose accused `codegen.rs`'s
   element-materialise arm, whose comment *discusses* a pre-Set free.

⚠ **And the check was VACUOUS until a probe caught it.** Deliberately removing
`!is_skip_free(v)` from the loft#723 site — the exact regression the rule exists for — did
not fire it: for a `let NAME = <cond>;` the region collected the *lines mentioning* `NAME`,
while the free lives inside the block one of those lines opens. Fixing that made the probe
fire **and immediately turned up a real site the earlier version had hidden.**

**The site it found:** `generation/dispatch.rs`'s `owned_ref_reassign` freed on the proxy
with no override — while its own comment says it mirrors "the interpreter's predicate", and
the interpreter's twin has consulted the override since loft#723. Two backends reading
different facts for the same decision is what `O-NoDiverge` exists to forbid.

**Latent, and said so.** No shape reproduced a fault — including the `??`-materialiser
shape the code names, under `LOFT_POISON` and `LOFT_STRICT_STORES` and the native leak
check. Closed anyway, because the rule is what says which fact a free may read, and the
asymmetry between backends is a defect in its own right.

⚠ This qualifies a **CLOSED** deviation. `D-own-1` (closed 2026-07-04) records *"the LAST
per-site ownership re-derivation is now GONE"* and *"every free/copy/move reads `deps`"*.
Both remain true in the letter — these sites do read `deps`. What is not true is the
implication that reading `deps` is sufficient: since loft#723 it demonstrably is not, and a
second fact carries the difference. Re-opening `D-own-1` would misdescribe it; the honest
record is this note plus the two code sites now saying so in their own docs.

### heap.md — the runtime half of the lifetime map (2026-08-25)

`Stores::free` / `free_named` (`src/database/allocation.rs`) is the runtime free, and it is
the **third** load-bearing lifetime function found undocumented — after `get_free_vars`
(free placement) and `Scopes::scan_if` (the path reconcile). Now cited: `@FR-H-Free`, with
its guards in check order — `@FR-H-FreeNull`, `@FR-H-FreeStack` (a `#306` bug IS a
stack-record ref mistaken for an owned heap store), `@FR-H-FreeTwice`.

**A candidate divergence that was not one.** `H-FreeTwice` says a double free is a FAULT;
the code answers it with `if store.free { return; }` — a silent no-op. That reads like the
`D-Opt` shape, and it is not: `heap.md` states outright that the corrupting frees are *"not
re-checked at runtime — discharged statically"* by ownership.md's checker, so reaching one
means the static system already failed and `LOFT_POISON` is the cross-check that surfaces
it. Checking the rule's own prose before filing is what kept this from becoming a false
report.

⚠ **What WAS wrong is the soundness argument's footing.** `heap.md` asserted twice that
ownership.md is at **0 open deviations**, and `H-Sound` discharges the whole free discipline
onto that checker. ownership.md is at **`OPEN: 1`** — `D-own-8`, *"a Join's ownership fact is
true on one path only"*. That is a PATH-COMPLETENESS gap, and path-completeness is exactly
what `H-Sound` consumes, so the stale claim was wrong in the direction that matters. Both
occurrences now state the current register and say to re-read it rather than the sentence —
**a claim about another document's register goes stale silently**, which is the same failure
as `L-Tuple` naming a renamed function, one document further out.

## One notion, how many SPELLINGS? — the dual question, and its five instances (2026-08-26)

Everything above asks *"is this the same question asked twice?"*.  This asks the dual, and it
turned up five times in one week in five different subsystems — the last of them not in the IR
at all, but in the TYPE lattice:

> **A notion the language treats as ONE thing can reach the IR in more than one SPELLING, and a
> matcher keyed on one spelling is blind to the other — silently, because the missing spelling
> shares no token with the one it looks for.**

That blindness cannot be grepped for from the symptom.  A search for the spelling a predicate
DOES match returns every site that gets it right; the sites that get it wrong contain nothing to
search for.

| notion | spelling A — what the matcher looks for | spelling B — what it cannot see | what it cost |
|---|---|---|---|
| a **projection** | `Call(OpGetField \| OpGetVector, [base, …])` | `Value::TupleGet(base, i)` — a variant carrying its base as a var NUMBER, not a call at all | a tuple-element tail renamed the tuple local onto a vector-shaped `__retbuf`: null return on `--interpret`, refused outright on `--native` (QUALITY.md § B6e).  `is_projection_op(data, d_nr)` **cannot even express** spelling B — it takes a def number, and a non-call projection has none |
| a **null at a branch join** | the LITERAL, lowering to `OpConv*FromNull` / `OpNullRefSentinel`, which the five DN1 null-arm walkers match | a nullable-**TYPED** value — a `-> τ?` call, an index read — which is a null by TYPE and carries no null-shaped node | `x: integer = if k == 9 { 1 } else { maybe(k) }` compiles and `x` holds null, narrow widths included ([types.md](types.md) D-Null-Join, loft#1103) |
| a **borrow** | a value with a non-empty DEP list | a borrow with NO dep — `e = mk().items` views a `__lift_N` whose container dep loft#882/#889 record at the SUBSCRIPT only | the return promotion had to grow a leg that reads the DEFINING STATEMENT instead of the deps (loft#1101, [ownership.md](ownership.md) D-own-10) |
| a **projection**, again, one level out | the op NAMES a matcher happens to list — `OpGetField`, `OpGetVector` | `OpGetRecord` (a keyed lookup) and `OpVectorRef` (a linked element), which answer a `DbRef` in the same store by the same declaration `-> reference[data]` | `pick(h[k], …)` had no @P290 witness at hash, sorted and index alike, so every call orphaned the record the callee minted — one per call, both backends ([ownership.md](ownership.md) D-own-11) |
| a **nullable struct**, as a TYPE | `Optional(Reference(S))` — what `f: S?` parses to | `Enum(__nullable<S>, true)` — what `typedef::synth_nullable_struct_fields` rewrites the declared FIELD type to, in `fill_all`, BETWEEN the two parser passes | four failures from one root: `s = o.f` refused as a type change between one type and itself; `s = o.f ?? S { … }` reading the default at the payload's OFFSET; a VALUE default doing the same with no hint able to cure it; and `v[i] ?? S { … }` refused because the pass-1 `Some` build leaves its operand `Value::Null`, which the `?? null` check reads as a nullable fallback (QUALITY.md § B6j) |

**The instrument is per-notion and cheap.**  `scripts/ir_walker_audit.py spellings` asks it for
the projection notion: who resolves a projection by op name, and do they also carry a `TupleGet`
arm — **39 functions, 4 of them do**.  The mode is about thirty lines; the shape generalises to
any notion whose two spellings can be named.  Its first outside use found a latent blindness in
freshly-landed code (`expr_borrows_local` resolves by `OpGetField` / `OpGetVector` and cannot see
`TupleGet`; five tuple spellings answer correctly today only because the DEPS leg covers what the
structural leg cannot see — recorded on D-own-10).

⚠ **The fifth instance says the question is not only about the IR.**  A TYPE can have two
spellings for the same reason a node can — one is what the author wrote and one is what a
lowering produced — and the trap is sharper, because the two are separated by a PASS rather than
by a match arm.  Any rewrite that runs in `fill_all` puts the author's spelling on one side of it
and the lowered one on the other, so every cross-pass comparison of that type is a candidate.

**As a rule for writing one:** before you write *"is this an X?"* over the IR, ask whether X has
a second spelling — a `Value` VARIANT beside an op call, a TYPE beside a lowered TYPE, a TYPE
fact beside a node shape, an absence beside a presence.  Match the notion, not the spelling, and put both in ONE predicate.
In loft the variants carrying a var number outside `Value::Var` are `TupleGet`, `TuplePut`,
`CallRef`, `FnRef`, `FnRefDnr`, `Set` and `Iter`; `scopes::dominance_walk` names three of them and
is the model.

**As a rule for reading one:** a matcher that is *right* about every site it can see is the
normal appearance of this defect.  So the evidence is never a failing site — it is the notion's
other spelling, constructed by hand, arriving where the matcher is not looking.

⚠ **And the instrument has the same disease as its subject.**  It read 18 · 2 · 16 because it
knew two of the three ways Rust resolves an op here — `def_nr("OpGet…")` and a call to
`is_projection_op` — and not `data.def(d).name() == "OpGetField"`, which is how every
hand-spelled list in the tree is written.  Teaching it the third spelling took the count to
38 · 4 · 34 at the time and moved BOTH columns, which is what distinguishes precision from
noise.  The
fourth row above was found by hand, from inside one of the lists the screen could not see; a
mode that asks "who is blind to spelling B" is worth asking of the mode itself.

## Not mergeable — recorded so the question is not reopened

| the pair | why they must stay apart |
|---|---|
| `set_default_value_nullable` vs `Stores::is_null` | one WRITES through the encoding-aware setters, the other READS a raw walk. Folding them is the same false merge `validate_claims` was ruled out for (QUALITY.md Cluster C/D). |
| `copy_claims`'s `Parts::DbRef` skip vs `validate_claims` | measured 2026-08-22: both are exhaustive for their own question and the coupling is stated at the load-bearing end rather than shared. |
