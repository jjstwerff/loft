<!-- SPDX-License-Identifier: LGPL-3.0-or-later -->
# Diagnostic codes

Every diagnostic loft can raise should carry a **code** — a kebab-case slug printed in
brackets after the level:

```
advice[avoidable-copy]: copy of vector<integer> — `src` is still used after this point …
```

The code is the **frozen identity**; the prose is free to improve (@PLN102 arc-E E1). That
split is what lets four consumers share one handle:

- a **reader** searches for it;
- a **fix** attaches to it, rather than to a message string that drifts (@PLN131);
- **this file** is named by it — the door a fix's concept opens onto;
- `grep -rn "<code>"` finds the emitter, its tests and its documentation together.

loft uses **slugs, not numbers** (`avoidable-copy`, not `E0142`). A slug is self-describing
*and* greppable; a number means nothing until you have found the lookup table.

> **A code is a public surface, frozen once emitted.** Renaming one breaks every link and
> search that ever pointed at it, so the naming pass matters more than the edit. Assigning a
> code to a site that had none is additive and always allowed.

## The codes

The **fix** column says what `--explain` offers at that code: `M` a mechanical fix, `C` a
conditional one (each is one fix line), and the concept door they open onto.

| code | level | what it says | what to write instead | fix |
|---|---|---|---|---|
| `avoidable-copy` | advice | A structure was deep-copied because its source is still used after the copy site, so it could not be moved. | Build the value in place, or stop using the source afterwards. Both take the copy to zero — see `@F106`. | M C · `@F106` |
| `lost-write` | warning | A local was mutated but never read. A whole-value bind COPIES the heap value (C86), so the mutation landed in the copy and the write is LOST. | Bind a live reference with `&` for write-through, or read the local after the mutation if a copy was intended. | C C · `@F21` `@F106` |
| `text-parse-may-fail` | error | A text parsed `as <numeric>` can fail, and the result was typed non-null. | `?? <default>` for a fallback, `(… as T?)?` for the type's default, or `as T?` for a checked cast. | C C C · `@F2` `@F96` `@F5` |
| `cast-constant-out-of-range` | error | A constant does not fit the type it is bare-cast to, and a bare cast asserts that it does. | `?? <default>` for a fallback, or `as T?` for a checked cast. | C C · `@F2` `@F5` |
| `format-unescaped-brace` | error | A literal `}` inside a format string, where `}` closes a hole. | Write it `}}`. | M · `@F35` |
| `coalesce-default-type-mismatch` | error | A `??` default is not assignable where the value's type is expected. | Cast the default, or give it a matching type. | C · `@F2` |
| `shift-amount-out-of-range` | error | A constant shift outside `0..=63`, which has no defined result. | Shift by an amount inside the range. | C C · `@F37` `@F2` |
| `c-binding-not-interpretable` | error | A function bound to a C symbol with `#c` was called on the interpreter, which cannot make that call. | Run it on `--native`, or give the binding an interpretable path. | M M · `@F92` `@F53` |
| `superseded-call` | advice | A call to a `#superseded` symbol, from source you own. The old form keeps working — this is a signpost, never a removal. | Call the successor instead. | M · `@F109` |
| `unknown-function` | error | A call to a name that does not resolve, where a similarly-spelled function exists. | Rename to the suggested function. | M · `@F16` |
| `unknown-field` | error | A field or method that does not exist on the type, where a similarly-spelled one does. | Rename to the suggested field. | M · `@F12` |
| `unknown-variable` | error | A name that is not in scope, where a similarly-spelled binding exists. | Rename to the suggested binding. | M · `@F16` |
| `redundant-coalesce` | warning | A `??` whose left side is a non-null field — the default can never be used. | Delete the `?? <default>`. | M · `@F2` |
| `redundant-default-fallback` | warning | A `?` on a non-null field — the type default can never be used. | Delete the `?`. | M · `@F96` |
| `redundant-null-check` | warning | A comparison against `null` on a non-null field: the answer is fixed. | Delete the check, or compare the value you meant. | M C · `@F1` |
| `redundant-null-negation` | warning | `!x` on a non-null value — `!` tests presence, so it is always false. | Compare the value (`x == 0`) if that is what you meant. | C · `@F1` |
| `dead-assignment` | warning | A local is overwritten before it is read. | Delete the assignment, or read it before the next. | C · `@F100` |
| `never-read` | warning | A local or parameter is never read. | Delete it — for a parameter, that changes the signature. | C · `@F100` |
| `upper-case-local` | advice | An UPPER_CASE local — that style is reserved for constants. | Declare it `const`, or rename to lower_case. | C M · `@F18` |
| `unreachable-code` | warning | Statements after a terminator can never run. | Delete them. | M · `@F16` |
| `unreachable-match-arm` | warning | A match arm an earlier arm already matches. | Delete the arm. | M · `@F29` |
| `empty-parallel-block` | warning | A `parallel` block with no arms. | Delete it. | M · `@F33` |
| `text-slice-char-bound` | warning | A text slice ends at `len(text)` — a character count where a byte offset is wanted, so it stops short on multi-byte text. | `size(text)` for the byte length, or `text[i..]` for the rest. | M M · `@F97` |
| `text-index-char-bound` | warning | An index walks `0..len(text)` (characters) but reads bytes — silent on ASCII, wrong elsewhere. | Iterate `for c in text`, or walk bytes with `0..size(text)`. | M C · `@F97` |
| `index-bounds-other-vector` | warning | A loop bounded by `len()` of one vector indexes a different one — typed non-null, reads null on overrun. Opt-in (`LOFT_LINT_STRICT_INDEX`). | Bound the loop by the vector it indexes. | C · `@F6` |
| `function-complexity` | advice | A function's control-flow complexity passed the nudge, naming its deepest nesting line. | Lift the innermost part into its own function. | C · `@F16` |
| `too-many-parameters` | advice | A function takes many required parameters — every caller has to get them all right, in order. | Group the ones that travel together into a struct, or give the optional ones defaults. | C C · `@F12` `@F17` |
| `trailing-boolean-parameters` | advice | A function ends with booleans, so a call reads `f(…, true, false)`. | Give them defaults so callers pass only what they change. | M · `@F17` |
| `needless-reference-parameter` | warning | A `&` on a tuple parameter that is never written. | Drop the `&`. | M · `@F21` |
| `needless-const-parameter` | warning | `const` on a primitive parameter that is never modified. | Drop the `const`. | M · `@F18` |
| `slow-reference-parameter` | advice | A `&` parameter that is only read — double-indirect on every access, and field mutation already propagates without it. | Drop the `&` unless you reassign the whole binding. | C · `@F21` |
| `not-null-deprecated` | advice | `not null` is inert — a type is non-null by default now. | Delete it, or write `T?` if the type should allow null. | M C · `@F12` `@F1` |
| `const-reevaluated` | advice | A file-scope `const` is an inlined expression, so its initialiser runs at every reference. | Compute it once in a function that caches. | C · `@F18` |
| `digit-separator-grouping` | warning | Digit separators that are not on thousands boundaries. | Regroup in threes. | C · `@F3` |
| `empty-braces-not-collection` | warning | An empty `{}` where a collection literal was meant. | Write `[]`. | M · `@F6` |
| `divide-by-constant-zero` | warning | Division or modulo by a constant zero — the result is always null. | Divide by a value that can be non-zero. | C · `@F38` |
| `unary-minus-binds-tighter` | warning | `-x ** y` parses as `(-x) ** y` — the sign binds tighter than `**`. | Parenthesise as `-(x ** y)`. | C · `@F37` |
| `read-size-not-element-multiple` | warning | `f#read(n) as vector<T>` counts bytes, and `n` is not a multiple of the element width, so the tail is dropped. | Pass `element_count * <width>`. | M · `@F40` |
| `file-write-width` | warning | `f += <integer>` with no width cast writes 8 bytes; a binary file usually wants an exact width. | Cast to the width you mean (`as i32`, `as u8`, …). | M · `@F40` |
| `persist-bind-through-field` | advice | `store_persist_bind` on a collection reached through a field writes the whole container's store, which will not load back into a bare collection. | Bind a local of the collection's own type. | C · `@F40` |
| `missing-return-path` | warning | A function declared `-> T not null` has a path that returns nothing. Currently unreachable — a hard error fires first. | Return a value on every path, or declare `T?`. | C · `@F16` |
| `package-contract-drifted` | warning | A package was tested against an older loft contract than this one. | Ask its author to republish against the current contract. | C · `@F55` |
| `superseded-unknown-successor` | error | `#superseded "X"` names a symbol that does not exist, so the steer would ship dangling. | Name a real replacement, or drop the attribute. | C C · `@F109` |
| `superseded-not-folded` | warning | A `#superseded` symbol's body never calls its successor, so the steer ships without its fold. | Reimplement the superseded symbol as a shim over the successor. | C C · `@F109` |

**Every code offers a fix**, and `every_pinned_code_offers_a_fix` keeps it that way. When one
genuinely cannot yet, it goes in `FIX_BLOCKED` (`tests/e1_code_set.rs`) with the reason —
a listed exception, because a code that quietly ships without a fix looks exactly like one
that does not need one. That list is currently empty. It last held the `superseded-*` pair,
whose concept is `#superseded` itself: no catalogue entry meant no door, and a fix that links
nowhere is not finished. `@F109` cleared it, exactly as `@F106` had for `move`.

## Fix lines — `--explain`

A diagnostic says what is **wrong**. `--explain` (or `LOFT_EXPLAIN=1`) adds what to write
**instead**:

```
advice[avoidable-copy]: copy of vector<integer> — `src` is still used after this point …
  --> prog.loft:7:0
  |
7 |   h = Holder { v: src };
  | ^
  fix  build the value in place   [move · @F106]
  fix  drop the later use of `src`   needs: `src` is used again at line 8 — you do not need that   [move · @F106]
```

Three homes, no repetition: the diagnostic says what is wrong, the fix says what to write
instead, and the linked feature says why. A fix that re-explains the problem is duplication
the reader pays for every time; one that explains the concept inline has taken the
documentation's job. The concept (`move`) is a **handle** — the searchable noun that opens
the door — and `@F106` is the door.

**The rule cuts both ways, and the second cut is the one that costs.** A message may not
carry its own cure either. Most of these diagnostics used to (*"use `T?` for a checked cast,
or `?? d` for a fallback"*), so `--explain` printed the same advice twice. Moving it out is
what makes the three homes real — but it means a plain run now says only what is wrong, and
a reader who has never heard of `--explain` would simply be told less than before. One line
per RUN closes that:

```
error[cast-constant-out-of-range]: the constant 1e30 is out of range for `integer` — a bare cast asserts the value fits

note: 1 diagnostic above suggests what to write instead — re-run with `--explain`
```

Per **run**, not per diagnostic: a pointer under each one would double the output on a file
with fifty copy notices, which is the noise the opt-in exists to avoid. It disappears once
`--explain` is on — nobody needs to be told about a flag they just used.

**A message may only hand its cure away if a fix is there to catch it.** The copy notice
shows both sides: with a named source it has two fixes and the prose says only what is wrong,
and with no named source it has NO fix — so that branch keeps its resolution inline. The
alternative is a reader left with nothing at all.

**Two tiers, and they gate who may affirm the condition, not whether a fix is clickable:**

| | interactive (one click) | unattended (batch, CI) |
|---|---|---|
| **mechanical** — meaning fixed by the code alone | yes | yes |
| **conditional** — correct only if the stated condition holds | yes, the click IS the affirmation | **never** |

A conditional fix states its condition in its own column rather than inside a sentence,
because a click has to affirm it: the reader must **see** the thing being affirmed, not
extract it from a clause. That is also what gives the CLI and an LSP code action one shared
shape — `title` on the lightbulb, `condition` in the confirm step.

The condition names the surviving use by **line**. "`src` is unused after here" sends a
reader hunting; "`src` is used again at line 8" is affirmable in a second, and that
difference is the whole point of the field.

**Not every diagnostic has a mechanical fix.** The append shape (`dst.data += src.data`) has
no build-in-place rewrite at all, so a fix set is shape-dependent; assuming otherwise ships a
fix that does not exist. And a site whose condition an author can see is false should offer
**nothing** — suppressing a bad suggestion matters more than emitting a good one, because
credibility is what makes the click safe.

Four of the codes show what that costs in practice. `coalesce-default-type-mismatch` has an
obvious-looking rewrite — cast the default — that is **not** offered as an `edit`, because
`"x" as integer` is a text parse that can fail, so synthesising it would answer one error
with another. `c-binding-not-interpretable` offers two fixes and spells neither: one is C the
compiler cannot write, and the other is not a source change at all. `lost-write`'s two
ways out are BOTH conditional — the analysis proves the write is lost, never which of the
two the author meant, and a mechanical tier there would be a guess wearing the safe label.

`superseded-unknown-successor` withholds an `edit` for a reason worth keeping in view, since
it applies to any deletion fix: "drop the `#superseded` attribute" is a rewrite the compiler
knows exactly, but the diagnostic sits at the **definition**, not at the attribute's span —
so an `edit` there would tell an applier to delete the function. A fix may only spell an edit
it can also **place**.

**Ranking is on SOUNDNESS first; teaching is the tiebreak between fixes that both hold.**
Two codes make the distinction concrete.

`shift-amount-out-of-range` — `?? <default>` teaches an idiom and "use an amount inside
`0..=63`" teaches nothing, but a constant out-of-range shift is nearly always a wrong amount,
so the escape ranks second. The better lesson does not get to paper over a bug.

The two cast codes are the sharper case, and they cost a tier. `as τ?` is the idiom that
generalises, and it makes the expression **nullable** — which a target declared non-null
rejects, so `x: integer = "5" as integer?` does not compile. The parser cannot see that
target: applying the fix changes what pass 1 INFERS, so the same source that reads
`x: integer` before the rewrite reads `x: integer?` after, and only a re-parse knows. That
makes the checked cast **conditional**, not mechanical — its meaning is not settled by the
code alone — and it ranks below the discharging forms that hold wherever the diagnostic
fires. `loft fix` confirms it either way: `verified — yours to accept` where it holds,
`REJECTED` where it does not.

This is the general shape, and it is worth stating once: **a fix's tier is a property of the
evidence, not of how short the rewrite looks.** `lost-write`'s `d = &s.items` is one token
and still conditional, because the analysis proves the write is lost and never which
resolution was meant.

`--explain` never applies anything. `loft fix` is where a fix gets checked and written.

## Checking a fix by running it — `loft fix`

The compiler holds the analysis that raised the diagnostic, so a candidate rewrite can be
**applied to an in-memory copy and the analysis re-run**. That is what an IDE quick-fix
historically could not do, and it turns "this fix is sound" from an assertion into a
measurement.

```
$ loft fix prog.loft
prog.loft
  prog.loft:2  double the brace        [verified]
  prog.loft:7  make the cast checked   [REJECTED (the rewrite introduces an error)]

$ loft fix --apply prog.loft
prog.loft
  prog.loft:2  double the brace        [applied]
  wrote 1 fix(es) to prog.loft
```

A fix `Clears` only when the diagnostic's **code** is gone from the re-analysis *and* no
error appeared that the original did not have. Both halves are needed: a rewrite that
silences one error by causing another is not a fix, and that is exactly what a
pattern-matched suggestion produces. Matching on the code rather than the message is why the
code index landed first — prose is free to change, the code is not.

`--apply` writes a fix only when all three hold:

| gate | rejects |
|---|---|
| `Mechanical` | a fix whose correctness rests on a condition — an unattended run has nobody to affirm it |
| spells an `edit` | a fix that knows the rewrite but not where it goes |
| verifies | a fix that does not clear its own diagnostic, or that breaks something else |

**The program is never run.** The design asked for a behaviour comparison across both
backends; that would mean executing the author's code as a side effect of their asking what
to write — code that may write files, take a network turn, or not terminate. Verification is
static, and stops where acting on the author's behalf would begin.

`text-parse-may-fail` is the standing example of the third gate paying for itself.
`x: integer = "5" as integer` is offered the checked cast like any other failing parse, and
`as integer?` there yields `integer?` into a non-null slot — plausible, and refused by the
measurement. The same fix verifies where the target is not annotated, which is why it is
CONDITIONAL and ranked last rather than removed: it is the right rewrite most of the time,
and the author can tell in a second whether their own declaration allows it.

## In an editor

Three surfaces, and they carry different halves:

| LSP field | what it carries |
|---|---|
| `codeAction` | a quick-fix per fix that spells an `edit` — both tiers, since a click IS the affirmation. A conditional fix's title carries `— only if …` so the condition cannot be missed, and only mechanical ones are `isPreferred`, because a "fix all" gesture has nobody reading. |
| `relatedInformation` | **every** fix — title, condition, concept and door — as text under the diagnostic |
| `codeDescription` | the code's row in this file, as a link the editor renders on the code itself |
| `source.fixAll` | one edit applying every mechanical, VERIFIED fix — delegated to `fix_apply::apply_fixes`, the same function behind `loft fix --apply`, so the two lanes cannot drift |

**`relatedInformation` is the one that matters most, and it is easy to think redundant.** A
quick-fix needs an `edit`, and only 5 of 62 fixes have one — the rest name a rewrite the
compiler cannot place. Without this row an editor shows a message that, since the messages
stopped carrying their own cure, deliberately no longer says what to write: the cure would
live in the CLI alone, and the editor would be strictly worse off than before the trim.

The `codeDescription` link targets `main`, so it resolves from a release and 404s from a
branch that has not merged this file yet. That coupling is deliberate — a user reaches the
binary through a release cut from `main`, the same commit that carries this page.

**An UNMASKED error is not one the fix caused.** `parse_source` returns early when pass 1
errors, so a truncated parse reports no pass-2 diagnostic at all — casts, shifts, most
semantic lints. Fixing the pass-1 blocker lets the next parse reach them, and a plain
set-difference reads every one as damage the rewrite did. That made `--apply` and fix-all
refuse any file whose fix was not its only error.

Verification therefore compares the two parses only when they are **comparable**: when the
original got as far as the rewrite did. Where it did not, the fix is judged on its own
diagnostic alone — it cleared the blocker, and what the deeper pass then finds was always
there. Which is what a person does: fix the syntax error, then read the type errors.

Judging this by POSITION would have been wrong, and measuring said so before any code
changed: an unescaped brace hides a bad cast three lines below it *and* another two lines
above. The mechanism is the phase, not the line.

The gate still bites where a rewrite genuinely breaks something, because that failure lands
in the same phase the original reached: `x: integer = "5" as integer?` is still refused.
`a_genuinely_broken_rewrite_is_still_refused` is what keeps "ignore unmasked errors" from
becoming "ignore all errors".

**Errors get codes when they get FIXES.** The did-you-mean family is the model: each site
already computed a replacement and knew where the name sat, so the fix was a rename with a
real span — and coding them was justified by that, not by a coverage number. 500-odd parse
and type errors remain uncoded on purpose; a code is frozen the moment it ships, and minting
one for a message that will never carry a fix buys a permanent name for nothing.

**The spelling is a guess; the re-parse is what makes it a measurement.** A suggestion is
chosen by edit distance, which knows nothing about arity or type — `helpr` → `helper` can be
the right spelling and the wrong call. Verification catches exactly that, and it is the
reason these can be `Mechanical` and written unattended: a pattern match alone would have
edited the file and left it broken. It is also why the family reached the LSP years before
it could be applied — a quickfix a human clicks is a different risk from one a
fix-on-save applies.

## Adding a code

1. Emit through `Diagnostics::add_at_coded` (or `diagnostic!(… code = "…", …)`), never the
   uncoded `add_at`. Write the message as what is **wrong** — no "use X instead" clause; that
   is the fix's job, and a message carrying both is the duplication above.
2. Add the row to the table above **in the same change**. A code with nothing to grep to is
   the same dead door as a concept with no catalogue entry.
3. Attach the resolution with `Diagnostics::fix_last`, and give the concept a `@F` entry that
   exists. This is not optional: `every_pinned_code_offers_a_fix` fails a code that renders
   none, and `every_offered_door_resolves_to_a_catalogue_entry` fails a `@F` that is not in
   the catalogue. If the fix genuinely cannot be offered yet, add a row to `FIX_BLOCKED`
   naming what blocks it — a listed exception, never a silent one.
4. **Point the caret with `diagnostic_at!` whenever detection happens after parsing.**
   `diagnostic!` uses the lexer's CURRENT cursor, which is only right while the offending
   construct is still under it. A whole-function judgement (complexity, parameter count,
   trailing booleans) needs the whole body first, and by then the cursor sits on the NEXT
   definition — so all three of those pointed at the following `fn` while the prose named
   this one, which reads as advice about a function that is fine (loft#815). Pass the
   definition's own `position`. Same rule for anything checked after its expression is
   consumed. Guard: the golden case
   `tests/error_messages/cases/48_advice_points_at_the_function_it_names.loft`, whose
   fixture deliberately ends in a `next_function_marker` no caret may land on.

## See also

- [COPY_DIAGNOSTICS.md](COPY_DIAGNOSTICS.md) — the copy-vs-borrow model behind `avoidable-copy`.
- [plans/131-suggestions/](plans/131-suggestions/README.md) — @PLN131's closure record:
  what was decided, and what the build learned that the design had not.
- `@F110` — the user-facing capability in the feature catalogue.
- `@F106` — copy and move semantics, the feature the copy fixes open onto.
