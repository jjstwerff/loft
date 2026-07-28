<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# H12 — returning a projection INTO something the callee owns handed back nulls

> **Status: FIXED (2026-07-27).** Eight-cell matrix green on both backends with no
> leaks; full suite 3499/3499. Regression guard:
> `tests/scripts/h12-return-vector-element.loft`. The predicate it turns on is
> `Parser::return_projects_into_local` (`src/parser/control.rs`), which replaces the
> narrower `return_field_base_is_call` that H9 added.

Reported by moros (H12), where it aborted the editor (`SIGABRT`) on the first terrain
sample. Reduced, it returns nulls.

## The symptom

```loft
struct Cell { c_h: u16, c_m: u8 }
struct Bag  { b_cells: vector<Cell> }

fn get_elem(i: integer) -> Cell { b = make_bag(); return b.b_cells[i]; }
```

`--native`: every field of the result reads `null`. `--interpret`: correct. An
explicit `return` does not help — which is what distinguished this from **H9**, whose
explicit form already worked.

## Why it cost more than a wrong value

The result is *uniformly* null, so a consumer reads it as **absent** rather than
**broken** — and in a world model where an unwritten cell IS empty, a dead cell and an
empty one are indistinguishable. It surfaced as a crash only because the null reached
arithmetic; a codebase that tolerates nulls would have got a plausible, wrong, empty
world and no signal at all.

## Two gaps, not one

Both found by instrumenting the delivery decision rather than reading it:

1. **The explicit return matched no delivery arm.** A vector-element read types as
   `Optional(τ)` (`v[i]` is `τ?`), and every arm in `parse_return` matches a DENSE form
   (`Type::Reference` / `Type::Enum`). So nothing bound the value to `__retbuf`, and
   the IR came out as

   ```
   OpGetVector(OpGetField(b, 0, 68), 3i32, i);   ← evaluated, value DROPPED
   OpFreeRef(b);
   return null;                                  ← the actual return
   ```

2. **The implicit tail took the `Rename` fallback.** `classify_reference_delivery`
   received `ls = [b]` and `return_views_local` said *false*, because it inspects each
   dep's own **further** deps — `b` is an owned local with none — so it could not see
   that the tail merely projects *into* `b`. `Rename([b])` then promoted the local `b`
   (a `Bag`) onto a `Cell`-typed return buffer: the signature came out as
   `fn n_get_tail(i: integer, b: Cell) -> Cell["b"]`.

## The invariant

> *A `Rename` delivery is sound only when the tail **is** the work-ref. When it
> projects into one — a field or an element read — the record lives in a store the
> callee frees, so it must be copied into the caller's buffer first.*

`return_projects_into_local` asks exactly that question, for both projection kinds
(`OpGetField`, `OpGetVector`), through chains, rooted at either a non-argument local or
an inline call's temporary. An **argument**-rooted projection is deliberately excluded:
the caller owns that store, so the view outlives the call.

## Why the interpreter was right, and why that is not reassuring

The interpreter's `Return` picked the dropped value off the eval stack, where the
discarded read had just left it. It agreed with the correct answer **by coincidence of
stack layout** — any change to the eval-stack discipline would have broken it silently.
So cross-backend disagreement was the only available signal, and a single-backend run
looked clean.

## The class was wider than the report

Five shapes were broken; moros reported two. Measured against the installed
pre-change binary as the before-oracle:

| shape | before (`--native`) | after |
|---|---|---|
| element of a local's vector field, explicit `return` | `null null` | ✅ (reported) |
| the same as the implicit tail | `null null` | ✅ (reported) |
| element of a **bare local vector** (no struct field in the path) | `null null` | ✅ |
| element after a **nested field chain** | `null null` | ✅ |
| element of an **inline call's** vector | `null null` | ✅ |
| element of an **argument's** vector (control) | `77 9` | unchanged ✅ |
| the hand-written copy (control) | `42 2` | unchanged ✅ |
| a **field** of a local — H9's neighbourhood (control) | `2` | unchanged ✅ |

The report's "crossing a package boundary" turned out to be incidental: the fault
reproduces in a single file.

## Notes for the next investigation here

- **A warm cache made the instrument blind.** Parser `eprintln`s produced nothing on
  re-runs because the library was served from `.loft/cache` — deleting `.loft` in the
  package was not enough, only a virgin directory was. Any parser-level probe needs a
  fresh path (or all three caches cleared: `.loft/`, `~/.loft/build-cache`,
  `native-auto/`), or it silently reads a stale parse.
- **`return_field_base_is_call` is gone**, subsumed by the general predicate. The H9
  and H7 regression scripts were green on both backends before it was deleted.

## The third sibling: a returned vector field of a call (zero-trust § 12)

> **Reported and fixed 2026-07-27**, after the struct and element forms above.  Guard:
> `tests/scripts/zt12-return-vector-field-of-call.loft`.

`return f().field` where the field is a heap **vector** returned a view into the freed
lift temp — empty from the start on `--native`; on the interpreter a live value that a
later store-allocating call in the caller transiently clobbered to length 0.  It cost the
consumer a one-line accessor that silently corrupted its result.

**Why the H12 fix did not cover it.** The explicit-return **vector** arm selects its
return buffer with its own condition, and #488 had given it a **Var-only** predicate: a
field view rooted at a local `Var` copied into the buffer, but one rooted at a *call
temporary* fell through — so the field read was emitted as a discarded statement and the
function returned an empty buffer.  Exactly the signature of the struct case, one arm
over.  Swapping that predicate for `return_projects_into_local` unified all three
siblings on one question, and retired `return_field_base_is_local_var` the way the
struct fix retired `return_field_base_is_call`.

**The emitted shape needed no invention** — the working `l = f(); return l.lines` form
already produces it: clear the caller's buffer, append the elements into it, return the
buffer.  Its IR *was* the spec.

**The boundary is narrower than the report suggests**, and the controls are what pin it:

| shape | before | note |
|---|---|---|
| `return mk().lines` (struct elements) | `len 0` | reported |
| `return mk().nums` (`integer`) | `len 0` | element width is NOT an axis |
| `return mk().bytes` (`u8`) | `len 0` | — |
| implicit tail `mk().lines` | ✅ 2 | already handled by `block_result`'s #437 intercept |
| `return mk().inner.lines` (chained) | ✅ 2 | a chain is delivered via a materialised work-ref |
| `return f.lines` (argument-rooted) | ✅ 2 | the caller owns that store |
| `Holder { copy: mk().lines }` | ✅ 2 | why the consumer's render path never tripped |
| `return o` (fresh local, #437) | ✅ 2 | must keep NRVO — its block cannot sit in `OpAppendVector`'s arg |

The last two controls are load-bearing: a change that copied *every* vector return into
the buffer passes the failing cells and breaks the fresh-local return.

## See also

- `tests/scripts/h12-return-vector-element.loft` — the eight cells, three of them
  controls.
- [`../h7-retbuf-alias/README.md`](../h7-retbuf-alias/README.md) — the sibling
  consumer finding, same reporter, same session.
- moros `doc/claude/LOFT_HANDOFF.md` § H12 — the original report (read-only; theirs).
