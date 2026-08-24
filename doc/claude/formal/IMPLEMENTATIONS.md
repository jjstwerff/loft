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

## Prerequisite — rule TAGS, and what the measurement actually said

The checklist below, and any generated rule→site index, rests on being able to grep a rule and
get exactly its sites. Measured 2026-08-24:

| | |
|---|---|
| rules with a definition line in `formal/` | **356** |
| rule-like names appearing anywhere | 361 |
| **prefix collisions among DEFINED rules** | **23** |
| family prefixes used in prose that are NOT rules | 5 — `B-Ref`, `D-op`, `D-own`, `D-cap`, `D-op-null` |
| rules mentioned anywhere in `src/` | **83 of 361**, across 185 file-hits (a mention, not yet a citation) |

The real collisions are between a rule and its own sub-rules: `@B-View` / `@B-View-Base`,
`@T-Ref` / `@T-Ref-El`, `@P-Cap` / `@P-Cap-Fresh`, `@F-Target` / `@F-Target-Pos`. A plain
`grep B-View` sweeps in its sub-rules, and `\b` does not help — `-` is already a word boundary.

⚠⚠ **Two corrections to earlier numbers in this file, both measurement artifacts of mine, and
the second is the more embarrassing one.**

1. The first pass reported **0** prefix collisions. The rule-name regex could not span a second
   hyphen, so it split `B-Ref-Alias` into `B-Ref` + `Ref-Alias` and never saw a multi-hyphen
   name. A zero from an instrument that cannot represent the thing it is counting is not a
   measurement.
2. The corrected pass then reported **33**, headlined by *"`grep B-Ref` returns ten other
   rules"*. `B-Ref` **is not a rule** — it is a family prefix that only ever appears in prose,
   with no definition line. Counting mentions instead of definitions inflated 23 to 33 and put
   a non-rule at the top of the evidence. Separating DEFINED from MENTIONED is what fixed it,
   and it is the same lesson as the rest of this file: the instrument has to know what it is
   counting.

**The convention** is written up in [README.md § Rule tags](README.md) and
[CLAUDE.md § Tracker tags](../../CLAUDE.md): `@`-tag every rule, cite the tag at each
enforcing site, match with an explicit boundary (`@Name` not followed by `[-A-Za-z0-9]`) rather
than renaming the 23, and treat only a DEFINED rule as a citation target.

Until the citations exist, the site counts in the checklist below are **lower bounds you cannot
size**, and `scripts/rule_predicate_audit.py`'s shape-matching is the only working instrument —
which is precisely the weaker anchor the convention replaces.

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
| 2 | **does this own a store** (`Reference\|Vector\|Enum(_,true,_)`) | `heap.md` H-Alloc; `ownership.md` O-Owner | **30** | ☐ largest single list; check whether all 30 ask the same question before touching |
| 3 | **is this a KEYED collection** (`Hash\|Index\|Radix\|Sorted\|Trie`) | `collections.md` Col-Hash / Col-Index / Col-Sorted / Col-Trie | **12** | ☐ no named home; the catch-all audit already found these as "only a keyed collection has keys" guards |
| 4 | **is this a collection** (keyed `+ Vector`) | `collections.md` Col-Store | **10** | ☐ differs from #3 by exactly `Vector` — confirm the two are genuinely different questions and not one drifted list |
| 5 | **is this a DbRef-represented type** (`+Radix\|Trie` over #2) | `layout.md`; `element_stack_size`'s DbRef group | **8** | ☐ `coalesce_not_null`'s heap-DbRef branch is one of these — the branch loft#1065 recursed into |
| 6 | **the narrow integer widths** (`Byte\|Int\|Short\|ShortRaw`) | `layout.md` L-Null; `types.md` narrow widths | **7** | ☐ partial homes exist (`write_narrow_value` / `narrow_is_null`); the remaining sites still spell the list |
| 7 | **what TERMINATES a block** (`Drop\|Return\|Yield`, `+BreakWith`) | `calls.md` F-Drop / F-Block | **9 + 13** | ☐ two variants; establish whether `BreakWith` belongs to the same question |
| 8 | **which `Value` shapes hold a statement list** (`Insert\|Parallel\|Tuple`) | `operational.md` | **8** | ☐ a `Value` list rather than a `Type` list; the audit script only reads `Type::` today |

## Not mergeable — recorded so the question is not reopened

| the pair | why they must stay apart |
|---|---|
| `set_default_value_nullable` vs `Stores::is_null` | one WRITES through the encoding-aware setters, the other READS a raw walk. Folding them is the same false merge `validate_claims` was ruled out for (QUALITY.md Cluster C/D). |
| `copy_claims`'s `Parts::DbRef` skip vs `validate_claims` | measured 2026-08-22: both are exhaustive for their own question and the coupling is stated at the load-bearing end rather than shared. |
