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

An editor gets the same fixes as code actions (`data.fixes` on the published diagnostic, with
each fix's own span). Both tiers are offered, because a click IS the affirmation — a
conditional fix's title carries `— only if …` so the condition cannot be missed, and only
mechanical fixes are marked `isPreferred`, since a "fix all" gesture has nobody reading.

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

## See also

- [COPY_DIAGNOSTICS.md](COPY_DIAGNOSTICS.md) — the copy-vs-borrow model behind `avoidable-copy`.
- [plans/131-suggestions/](plans/131-suggestions/README.md) — the suggestions design.
- `@F106` — copy and move semantics, the feature the copy fixes open onto.
