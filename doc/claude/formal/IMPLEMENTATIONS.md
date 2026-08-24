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
| 7 | **what TERMINATES a block** (`Drop\|Return\|Yield`, `+BreakWith`) | `calls.md` F-Drop / F-Block | **9 + 13** | ☐ two variants; establish whether `BreakWith` belongs to the same question |
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

## Not mergeable — recorded so the question is not reopened

| the pair | why they must stay apart |
|---|---|
| `set_default_value_nullable` vs `Stores::is_null` | one WRITES through the encoding-aware setters, the other READS a raw walk. Folding them is the same false merge `validate_claims` was ruled out for (QUALITY.md Cluster C/D). |
| `copy_claims`'s `Parts::DbRef` skip vs `validate_claims` | measured 2026-08-22: both are exhaustive for their own question and the coupling is stated at the load-bearing end rather than shared. |
