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

| code | level | what it says | what to write instead |
|---|---|---|---|
| `avoidable-copy` | advice | A structure was deep-copied because its source is still used after the copy site, so it could not be moved. | Build the value in place, or stop using the source afterwards. Both take the copy to zero — see `@F106`. |
| `lost-write` | warning | A local was mutated but never read. A whole-value bind COPIES the heap value (C86), so the mutation landed in the copy and the write is LOST. | Bind a live reference with `&` for write-through, or read the local after the mutation if a copy was intended. |
| `text-parse-may-fail` | error | A text parsed `as <numeric>` can fail, and the result was typed non-null. | `as T?` for a checked cast, `?? <default>` for a fallback, or `(… as T?)?` for the type's default. |
| `cast-constant-out-of-range` | error | A constant does not fit the type it is bare-cast to, and a bare cast asserts that it does. | `as T?` for a checked cast, or `?? <default>` for a fallback. |
| `format-unescaped-brace` | error | A literal `}` inside a format string, where `}` closes a hole. | Write it `}}`. |
| `coalesce-default-type-mismatch` | error | A `??` default is not assignable where the value's type is expected. | Cast the default, or give it a matching type. |
| `shift-amount-out-of-range` | error | A constant shift outside `0..=63`, which has no defined result. | Shift by an amount inside the range. |
| `c-binding-not-interpretable` | error | A function bound to a C symbol with `#c` was called on the interpreter, which cannot make that call. | Run it on `--native`, or give the binding an interpretable path. |
| `superseded-unknown-successor` | error | `#superseded "X"` names a symbol that does not exist, so the steer would ship dangling. | Name a real replacement, or drop the attribute. |
| `superseded-not-folded` | warning | A `#superseded` symbol's body never calls its successor, so the steer ships without its fold. | Reimplement the superseded symbol as a shim over the successor. |

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

Nothing is ever applied. `--explain` shows; the author decides.

## Adding a code

1. Emit through `Diagnostics::add_at_coded` (or `diagnostic!(… code = "…", …)`), never the
   uncoded `add_at`.
2. Add the row to the table above **in the same change**. A code with nothing to grep to is
   the same dead door as a concept with no catalogue entry.
3. If the diagnostic has a known resolution, attach it with `Diagnostics::fix_last` — and
   give the concept a `@F` entry that exists.

## See also

- [COPY_DIAGNOSTICS.md](COPY_DIAGNOSTICS.md) — the copy-vs-borrow model behind `avoidable-copy`.
- [plans/131-suggestions/](plans/131-suggestions/README.md) — the suggestions design.
- `@F106` — copy and move semantics, the feature the copy fixes open onto.
