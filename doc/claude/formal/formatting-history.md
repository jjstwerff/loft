# formal/formatting-history.md — the deviation register for [formatting.md](formatting.md)

> **The rules are next door.**  [formatting.md](formatting.md) states what must always be true of the
> language; this file is its TIMELINE — every place the code was measured not to do it, when,
> what it cost, and what closed it.  The two are apart because a contract a reader has to skim
> past its own history stops being a contract they can skim.  The rules doc carries the CURRENT
> state (how many are open, and which); everything below is the record behind it.

OPEN: **2** — `D-fmt-2` and `D-fmt-3` below, both opened 2026-08-29. `D-fmt-4` was opened and
closed the same day by the `@FR-E-NullArg` walk.

⚠ **This doc read `OPEN: 0` for its whole life, and the walk that first asked found four
defects.** The line was never a measurement: it said *"a rules doc adds no code deviation"*,
which is a claim about the doc's GENRE rather than about the code, so no oracle stood under it.
The four are `D-fmt-1` (closed) and the two open entries; what they have in common is that a
neighbouring spelling of each was already correct, which is what a differential oracle is
blindest to — both backends agreed, and agreed on the wrong answer.

### D-fmt-1 — OPENED AND CLOSED (2026-08-29): four ways a spec did not reach its renderer

Found by walking `@FR-F-Spec`; all four fixed in the same pass, guards in
`tests/scripts/a-format-spec-is-honoured-not-dropped.loft`,
`a-json-spec-spelling-is-one-decision.loft` and
`a-format-spec-the-renderer-cannot-execute-is-refused.loft`.

1. **A tagged null ignored its alignment.** `{a / b:<12}` right-aligned `null(/0)` while
   `{n:<12}` left-aligned a bare `null`, so the alignment a hole got depended on whether its
   null carried a fault cause. Six lines existed in THREE copies — `ops::format_long_with_tag`
   and the interpreter's `State::format_int` / `format_stack_int` — each passing a literal `1`
   where `format_long`'s bare-null path resolves `dir`. The two interpreter copies now call the
   shared one, so the question has a single place to be answered.
2. **`{p:J}` and `{p:json}` were not `{p:j}`.** Whether a WIDTH expression starts here and which
   radix a letter names are one question, and they were answered from two lists: the radix reader
   lower-cased and accepted `json`, the skip list named only `j`. The width expression therefore
   ate the letter — an "Unknown variable 'json'" where no such variable exists, and where one
   does, that variable's value silently taken as the width with the rendering falling back to the
   compact loft form. `Parser::radix_for` is now the one home both readings consult.
3. **A width that is not a number was accepted.** `{n:0>5}` parses as `0 > 5`; the BOOLEAN
   reached the width, rendering with no padding on `--interpret` and reaching rustc as
   `E0308 expected i64, found bool` on `--native`. This is the residual of the fix that made
   `string_states` order-independent — its own note says an out-of-order flag "was simply left in
   the stream for the WIDTH expression to find", and a `0` fill is left there for the same reason,
   because the fill branch can only claim a token and a digit lexes as an Integer. F-Spec-Fill
   above now says why a digit cannot be a fill; the width slot now requires an integer.
4. **`{n:e}` and `{n:j}` aborted the interpreter.** `ops::format_long` implements four radixes
   and ends in `panic!("Unknown radix")`; the spec reader answers two more. An ordinary source
   program reached that panic. Both are refused at the type dispatch now.

### D-fmt-4 — OPENED AND CLOSED (2026-08-29): a null character carried a fault cause, on one backend

- Was: a null `character` in a format hole rendered `null(<tag>)` when the fault tag was armed
  — `"hi"[9]` read `null(oob)` — where `(F-Render)` says a null character renders as NOTHING,
  and says why: iterating text past its end must append no garbage. Only the INTERPRETER did
  it (`State::append_character`); `--native` rendered nothing, so the two backends disagreed
  on an ordinary text overrun and `@FR-D-op-1` makes that a bug in whichever one disobeys.
- The intent was good and is worth restating: it showed *what* produced the missing character
  instead of an empty space. But a diagnostic that exists on one backend is not a language
  feature, and no test pinned it (the four `fmt43_*` cases in `tests/runtime_warnings.rs` are
  all INTEGER holes, and all still pass).
- Fixed toward the rule and toward native: `append_character` drops the tag rather than
  rendering it. The tag is still TAKEN, so it cannot leak into a later hole of the same
  string. Guard `tests/scripts/a-fault-tag-names-the-fault-that-happened.loft`.
- The cause itself is now written by the op that FAULTS (`Stores::note_format_fault`) rather
  than armed from the op's shape, so `null(<reason>)` names what happened — loft#1169. A hole
  may hold several fault-prone ops while only the outermost is armed, so a peer that inherits
  a null LEAVES the tag alone: the cause travels with the null from wherever it was born.
- **Reversing this is a change to `(F-Render)`, not to the op.** If a character hole should
  carry its fault cause, the rule's character row says so and BOTH backends implement it.

### D-fmt-2 — OPEN (2026-08-29, loft#1165): a `character` hole drops its whole spec

`{c:>5}` renders `x`. `Type::Character` emits `OpAppendCharacter`, which takes the accumulator
and the value and has nowhere to put width, alignment or fill. Violates F-Spec-Exec: it neither
honours the spec nor refuses it. Needs a cast-to-text op or a scratch-buffer lowering, plus the
decision F-Render forces — a null character renders as nothing, and a width would pad that to a
full field.

### D-fmt-3 — OPEN (2026-08-29, loft#1166): a vector/struct hole drops width and alignment

`{v:>12}` renders `[1,2]`. The record arms pass `OutputState::db_format()`, which is two bits
(`#` and the JSON radix); width, alignment and fill are not passed at all. Violates F-Spec-Exec
for the same reason and wants `OpFormatDatabase`'s signature widened on both backends.

- **Conformance is differential** — formatting is enforced across the two backends by the @PLN89
  oracle (D-op-1) plus the dedicated `tests/scripts/14-formatting.loft` / `tests/docs/30-formatting.loft`
  suites, which pin the exact rendered strings and run on both backends. A divergence in a rendered
  value, a null form, a struct/vector Display, or a format-spec result is caught there.
- **Rendering `null` as the word `null` is the contract, not a gap** — the in-band sentinels
  (`i64::MIN`, `NaN`, `STRING_NULL`, `0xFF`) are an operational decision (operational.md E-Null);
  their uniform `"null"` text (character-0 excepted) is intended, so it is a rule here, not a
  deviation.
