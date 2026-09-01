# Reference review — validating what we promise

The language reference is the promise. It ships inside **all four release bundles**
and on the docs site, and a reader treats it as the definition of the language — so a
sentence in it that is no longer true is not a documentation bug, it is a promise we
are breaking, quietly, to everyone who reads it offline.

**This pass is by hand, and that is not a gap to be closed.** `make release-checklist`
already answers everything a program can about the reference:

| check | what it establishes |
|---|---|
| `A-pdf` | built after every input that decides its content moved |
| `A-pdf-version` | the artifact says it is *this* release, read from its own bytes |
| `A-pdf-content` | every chapter is present, the stdlib chapter is not empty, no placeholder shipped |

All three can be green on a reference that describes behaviour the language stopped
having two releases ago. They check that the document is **whole and current**; not one
of them reads a sentence. What is left is the part that matters most to a user, and it
needs a person.

## Do it early — the watermark

Left to tag day this becomes a day of reading under time pressure, which is how a
review turns into a skim. So it is **continuous**: each chapter records the commit it
was last read through, and it only comes back on the list once its own source has moved
past that. Read a chapter the week its topic changes and the tag-day list is short
because the work already happened.

```bash
make reference-review              # the worklist: never-reviewed, and moved-since
make reference-review ARGS=--verbose   # + the commits behind each moved chapter
```

The aid reports three things and judges none of them: chapters with **no watermark row**
(never read), chapters that have **MOVED** since the commit their row records (with the
commit count, so "three commits, all typo fixes" is dismissed in seconds), and **stale
rows** — a watermark naming something that is no longer a chapter, which would otherwise
read as coverage we do not have.

## What to actually check in a chapter

The examples are **already gated** — every code example in the reference is a
`tests/docs/*.loft` file that runs in the suite, which is what lets page 1 claim it. Do
not re-verify them by hand; that budget belongs to the things no test covers.

⚠ **Read "example" narrowly: it means the code the file EXECUTES.** A chapter is mostly
comments, and three kinds of claim inside them run under nothing at all — which is where
every defect the first passes found was living:

| claim in a comment | what checks it |
|---|---|
| a code snippet shown but not executed (`fn f(v: &vector<T>) …`) | nothing |
| a table of results (`1.0 / 0.0 is inf`) | nothing |
| a quoted diagnostic ("a text parse `as integer` may fail…") | nothing — and an `assert` never can, because the program does not compile |

So the first move on a chapter is to separate the sentences the suite is holding up from
the sentences it is not, and spend the read on the second set.

**Where the proof goes once you have it.** Not into the chapter. A chapter is read by
someone meeting the subject for the first time, so proving a boundary exhaustively on the
page costs them more than it gives: keep the one or two cells that carry the LESSON and
move completeness to a guard in `tests/scripts/`. The two written for the Float and
Functions passes are the model —
`the-reference-float-boundary-is-where-it-says.loft` and
`the-reference-quotes-its-refusals-word-for-word.loft` — the second using `@EXPECT_ERROR`
cells, which one file can hold beside a running cell since loft#1242. Such a guard is a
LOCK rather than a regression test: it has no build on which it fails, so it records
`@falsified-at: none` and must be checked by hand in both directions, because a guard made
of expected failures passes most easily when it is proving nothing.

⚠ **Check a multi-cell `@EXPECT_ERROR` guard ONE CELL AT A TIME, in its own file.** Every
declaration after the first firing one is credited without matching (loft#1261), so the
whole-file run and `make falsify` both report `n/n` on a build that produces one of them —
measured on a guard whose four interesting cells all failed correctly when run alone.
Until that is closed, the by-hand check is: split the cells out, run each against the
build the guard was written for AND against this one, and record the two answers per cell
in `@falsified-at`. `const-binds-through-every-append-route.loft` carries that table.

⚠ **Read the chapter's MODEL before its claims.** Chapter 17 said a library name needs
its `libname::` prefix, and every section after that inherited it: the struct section
called the bare form a parse error, the free-function section called the prefix required,
and the import section built a distinction between `use lib;` and `use lib::*` that does
not exist. Reading claim by claim catches none of that — each sentence agrees with the
others, and the page only comes apart against the compiler. **One measurement did it: a
bare call answering.** So before the sentence-level read, take the chapter's central
rule, write the smallest program that would violate it, and run that.

⚠ **Check the LIBRARY CATALOGUE for the chapter's own subject.** Chapter 22 opened "Loft
provides two time functions" and told the reader to use `now()` for "date calculations" —
while a published `time` package does proleptic-Gregorian arithmetic on exactly the
millisecond integer `now()` returns (`format_iso(now())`, `weekday_name(now())`,
`add_days(now(), 30)`). The sentence is true about the standard library and reads as true
about loft, and the reader it fails is the one who believes it and writes calendar maths by
hand. CLAUDE.md already says to check `make libcatalogue` before building something; a
chapter is where that check is owed to somebody else. **Run it for every chapter whose
subject a package could plausibly cover**, and say plainly which side of the line the
chapter is on.

⚠ **A chapter whose subject is CONFIGURED needs a configured run, and that cannot live on
the page.** Chapter 20 documented a log file format, four config sections, per-file level
overrides, rate limiting and a production mode — and its executable cells called the four
log functions with NO config present, where the documented behaviour is to do nothing. So
the chapter asserted that logging is off, and every claim about what logging DOES ran under
nothing. Three were wrong, including the config spelling that `loft --generate-log-config`
prints into its own template. A `log.conf` cannot be added beside the chapter either — it
would switch logging on for every other chapter in the suite. **The home for a configured
chapter's proof is a Rust test that writes a private directory per case and runs the binary
in it**; `tests/logging_config_is_what_the_chapter_says.rs` is the shape.

⚠ **A chapter that states a rule does not say who KEEPS it, and the reader assumes the
compiler does.** Chapter 19 listed three worker rules in one bulleted list: two the compiler
enforces with precise refusals, and *"must not use global state or I/O (no println, no file
access)"*, which it does not enforce at all. `(C-Impure)` makes that one the author's
contract by design, so the language is right and the page was wrong to shelve it beside the
other two. Reading alone cannot separate them — running the violating program can, and the
cost of not knowing is a worker that appends three lines to a file and leaves one, a
different one per run. **For every "must" in a chapter, write the program that disobeys it
and find out which of the three answers you get: a refusal, a defined behaviour, or
silence.**

⚠ **And run the violating program even when you expect it to confirm the chapter** — the
chapter is not the only thing it can falsify. Chapter 18 taught a single-axis `const`
(the language has had two axes since @PLN40), which is a page defect like chapter 17's.
But the same little programs, run across the shapes the chapter does NOT mention, found
that `p: & const vector<T>` and `p: const hash<R[k]>` both accepted an append that reached
the CALLER — a `const` promise silently not holding, on both backends, which no amount of
reading could have produced. **The reference pass is a language probe that happens to
start from the prose**; the chapter's own subject is the axis nobody has swept.

⚠ **For every "cannot", "is an error" and "does not" in a chapter, write the program that
does it anyway.** Chapter 23 is a catalogue of traps, and its wrong claims CLUSTER: **five
of them are a stated NEGATIVE the language does not have**. Hashes "cannot be iterated"
(`for e in h` has walked one in ascending key order since C60, four and a half months
before the read), slicing inside a multi-byte character "is an error" (it rounds outward to
whole characters), `lines()` "crashes" on invalid UTF-8 (it warns, answers null, and hands
back an empty vector — the dangerous direction, because a reader defending against a crash
writes no defence at all against a silently empty read), a non-`&` vector parameter's
append "is local" (it reaches the caller — the exact claim loft#1251 corrected in LOFT.md
and the Vector chapter, still standing here because nobody had read this page), and
`e#remove` "inside a filtered loop" (it works in a plain one too). That is not a
coincidence about this chapter: **a negative is the one claim an example cannot carry**, so
the sentences a chapter's own cells hold up are exactly the positive ones, and everything a
page says the language will NOT do runs under nothing at all. The reference names the same
feature in more than one chapter, so this is also where a corrected claim goes to survive:
grep the whole reference for a sentence you fix, not just the chapter that owns the subject.

⚠ **A trap catalogue must say what it does not cover.** Chapter 23 opened by claiming it
"catalogues every known trap", which the shipped lints falsify by themselves — nothing on
the page about `omitted-field-zero`, `variant-field-unchecked` or the copy-on-bind lost
write, all of which the compiler now diagnoses. A closed-list promise ages into a wrong one
the first time the language grows a corner, and a reader who believes it stops looking.

⚠ **A CAVEAT outlives the gap it describes, and it costs more than a stale feature
claim.** Chapter 24 told the reader that `Type.parse(text)` "DROPS diagnostics — malformed
input and schema mismatches leave the struct at its defaults with `json_errors()` empty",
and sent them to a two-step spelling for error reporting. That gap (Q1) closed on
2026-08-20; the warning outlived it in THREE documents — the chapter, CAVEATS.md and a
ROADMAP row still asserting both halves — while QUALITY.md recorded the close. A stale
caveat steers a reader away from the correct, shorter spelling permanently, and nothing they
can run contradicts it: a caveat is a claim about what does NOT happen. So when a gap is
closed, grep for the WARNING as well as for the behaviour, and when you review a chapter,
treat each of its caveats as a claim to re-measure rather than as context.

⚠ **When two sentences in one chapter disagree, run BOTH — never pick the plausible one.**
Chapter 24 said schema mismatches "land as the loft null sentinel in the struct" and, forty
lines later, "never by putting a null in a slot the declared type says cannot hold one". It
also carried the caveat above beside a cell comment saying that same form reports. Both
disagreements were the tell that behaviour had moved and one half of the page had been
updated — the later, more specific sentence was right each time, but that is a pattern and
not a rule, and the only way to know is to run them. A chapter that contradicts itself is
the cheapest review lead there is: it has already told you where to look.

⚠ **A chapter's table may be a copy of a DESIGN doc rather than of the code.** Chapter 25
listed six built-in interfaces and what each one lets you write, and three rows were wrong
in the permissive direction — `Addable` "addition and subtraction" (`-` is refused),
`Numeric` "all four scalar operators" (`+` and `/` are refused), `Scalable` "multiplication
by a float factor" (a `scale` METHOD taking an INTEGER, answering `integer`, and satisfied
by no built-in type at all). Every row was a faithful copy of INTERFACES.md's
*"Phase 1 defines these interfaces in `default/01_code.loft`"* block — which is the design
as written, not the file as shipped. So the chapter was not careless; it trusted a sibling
doc that made a claim ABOUT A FILE without being checked against it. **When a chapter's
table names a code artefact, diff it against that artefact, and fix the doc the table came
from too** — otherwise the next chapter copies it again.

⚠ **A cell that only asks "does this compile" passes on a wrong answer.** Writing the
completeness guard for that table, the cell for `a - b` under `Numeric` was built from a
probe that had reported "compiled" — and asserting its VALUE showed it answers `-a`, with
the second operand discarded, on both backends (loft#1274). A bound is a promise about what
compiles, so a compile check feels like the whole of it; it is half. **Assert the value of
every operation a bound is said to permit**, because an operator that binds to the wrong
overload compiles perfectly.

⚠ **A chapter's general sentence is only tested at the types its examples use.** Chapter
26 said closures capture "at the moment the lambda is written … if a variable changes after
the lambda is written, the lambda still sees the original value", with no type on it. That
is true of every cell on the page — all of them scalars and text — and false of every other
type the sentence covers: a captured vector, hash, sorted, index or struct is SHARED, so a
later append reaches the closure and a write inside it reaches the enclosing scope. Nothing
on the page was wrong; the sentence generalising from it was. The chapter also said nothing
about the third regime, a scalar the closure WRITES to, which is shared rather than copied
and is how an accumulator is written. **So for any claim of the form "a variable …", list
the types the language offers in that slot and run one cell per type** — the claim's truth
is a function of the type, and a chapter's examples reliably pick one of them.

⚠ **And re-check the two COMPARISON pages, where the same claim is stated more strongly.**
`00-vs-rust.html` and `00-vs-python.html` exist to draw a contrast, so they restate the
reference's claims in their sharpest form — and they had the same wrong generalisation with
the hedges removed: *"Capture is always by value"*, and *"later mutations to the original
variable do not affect the lambda **(and vice versa)**. Python's `lambda` … closes over
variables by reference, so mutations are shared."* That draws the loft/Python line on exactly
the axis where the two agree, and it is aimed at the reader most likely to write an
accumulator closure. The vs-Rust page also listed "closures can be stored in structs" as a
Rust advantage, which loft has (a struct FIELD holds a capturing closure; only a COLLECTION
refuses one). These pages are their own review rows, but a claim corrected in a chapter is
owed a grep across them the same day.

⚠ **A section TITLE is a claim, and it is not held up by the cells under it.** Chapter 26's
"Closures with higher-order functions" contained no higher-order function — its cell defined
a capturing closure and called it directly. The capability is real (`map(nums, scale)` with
`scale` capturing works on both backends), so nothing failed and nothing said the section was
not showing its own subject. Read the titles as a list, on their own, and ask of each one
what cell would falsify it.

⚠ **A chapter demonstrating a feature ONCE pins the shape that works, and the suite then
guards the gap.** Chapter 27 shows `yield from` with a single sub-generator, `inner_vals()`,
which takes no arguments. Give the sub-generator an argument and `--native` emits Rust that
does not compile (loft#1277) while the interpreter runs it — so the chapter, the doc suite and
the native gate were all green on a feature broken for every parameterised delegation. The
single cell was not wrong; it was unrepresentative, and being executable made that invisible.
**For a feature the chapter demonstrates once, list the argument it varies and run the other
values** — here: does the sub-generator take arguments, is it nested, does it yield nothing.

⚠ **When the claim is about a NEGATIVE resource fact, first ask which instrument could see
it.** Chapter 27 promised "the abandoned generator frame is freed automatically", and the
store-leak gate is structurally blind to it: coroutine frames live in a side-table on `State`,
outside the store system on purpose, so a clean run proves nothing about the claim. The witness
had to be built — give the generator a store-backed local, so a retained frame retains a store
— and the property is an INVARIANCE, not a threshold: `peak` must not move with the number of
abandoned generators while `allocs` scales with it. A threshold alone passes on a pooled leak,
and an exit-time leak count cannot see growth at all. `tests/coroutine_matrix.rs
::an_abandoned_generator_frame_does_not_accumulate` is the shape, including the measurement
that proves the channel moves (three frames live at once reads `peak=5` against `3`).

⚠ **An internals doc can carry an example that does not COMPILE, and nothing will say so.**
COROUTINE.md § "Exhausting a generator early" documented ending a generator with `return;`,
with a worked example — while `formal/coroutines.md` (G-Return) says a generator has no
`return` and the compiler refuses all three spellings (`return;`, `return e`, and a body whose
tail is a value) with precise messages. The formal rule and the code agreed; the design doc had
simply not been re-read since the rule landed. Reference chapters are gated because they RUN;
`doc/claude/*.md` snippets are not, so **when a chapter's subject has a companion internals doc,
paste its examples into a file and run them** — and when the two docs disagree, the formal rules
settle it.

⚠ **The blind spot repeats one chapter later in a different variable, so name the variable
each time.** Chapter 26's general sentence was only tested at the TYPES its examples used;
chapter 28's "possibly different types" was the same defect with every cell all-`integer`, and
its "a value parameter gets its own copy" section called no function at all. Writing the missing
cell found loft#1278 — assigning to a `text` element of a by-value tuple parameter does not
compile on `--native`, while the interpreter gives the documented answer. `formal/tuples.md`
had already written the lesson down about ITSELF: its `OPEN: 0` read clean while loft#1004 and
loft#1005 were live because its oracle is all-`(integer, integer)` and carries no `text`, and
its deviations section says *"a conformance entry that names one member of a family is a claim
about that member, not the family."* **So: for each general sentence, name the variable it
quantifies over — type, arity, shape, position — and run one cell per value.**

⚠ **And that applies to the reviewer's own new cells.** Chapter 27 gained a section proving a
generator body is suspended, with a step counter — written with the loop shape that is lazy on
both backends. COROUTINE.md's CL-9 table says four other shapes still run the whole loop eagerly
on `--native`; measured, a generator with a statement AFTER the yield does 1000 body steps where
the interpreter does 5, values agreeing throughout. The new section was over-general in exactly
the way the chapter-26 entry above warns about, one chapter later, and had to be corrected after
it was committed. **A cell written to fix an over-general sentence is itself one example of one
shape** — check the caveat table for the subject before claiming the general case.

1. **Is every claim still true?** Not "does the example run" but "does the prose
   describe what the language does now". A behaviour that changed under a chapter that
   did not is the failure this pass exists for.
2. **Does it promise something we do not deliver?** A capability described in the
   future tense that never landed, a flag that was renamed, an idiom that now emits a
   diagnostic telling the reader not to write it.
3. **Is anything missing that a reader would look here for?** A feature shipped this
   cycle whose chapter was never extended is invisible to everyone who reads the
   reference rather than the changelog.
4. **Is the recommended idiom still the one we would recommend?** Advice ages faster
   than syntax. If a chapter teaches a pattern that a lint now flags, the chapter is
   wrong even though every sentence in it is accurate.

Record the read below. One row per chapter source, and the commit is the one you read
**through** — normally `HEAD` at the time.

## Watermarks

The one home for "reviewed through". `scripts/reference-review.py` parses this table;
there is deliberately no second machine-readable copy, because it would drift the moment
someone updated only the prose.

⚠ **A watermark names a COMMIT, and no rebase or cherry-pick preserves one.** Joining the
two checkouts rewrote the three rows below onto their twins, and until that was done the
worklist reported all three chapters as owing a re-read — each was being measured against a
commit unreachable from this branch, so its OWN review commit read as a change since the
review. Re-point the rows whenever commits are replayed; the tell is a chapter whose only
"commit since" is the one whose subject names that chapter.

| chapter source | reviewed through | commit |
|---|---|---|
| `tests/docs/01-keywords.loft` | 2026-08-31 | `e9643ff6` |
| `tests/docs/02-text.loft` | 2026-08-31 | `e9643ff6` |
| `tests/docs/03-integer.loft` | 2026-08-31 | `e9643ff6` |
| `tests/docs/04-boolean.loft` | 2026-08-31 | `e9643ff6` |
| `tests/docs/05-float.loft` | 2026-08-31 | `e9643ff6` |
| `tests/docs/06-function.loft` | 2026-08-31 | `e9643ff6` |
| `tests/docs/07-vector.loft` | 2026-08-31 | `e9643ff6` |
| `tests/docs/08-struct.loft` | 2026-09-01 | `e9643ff6` |
| `tests/docs/09-enum.loft` | 2026-09-01 | `e9643ff6` |
| `tests/docs/10-sorted.loft` | 2026-08-31 | `e9643ff6` |
| `tests/docs/11-index.loft` | 2026-08-31 | `e9643ff6` |
| `tests/docs/12-hash.loft` | 2026-09-01 | `e9643ff6` |
| `tests/docs/13-file.loft` | 2026-09-01 | `e9643ff6` |
| `tests/docs/15-lexer.loft` | 2026-09-01 | `e9643ff6` |
| `tests/docs/16-parser.loft` | 2026-09-01 | `e9643ff6` |
| `tests/docs/17-libraries.loft` | 2026-09-01 | `e9643ff6` |
| `tests/docs/18-locks.loft` | 2026-09-01 | `e9643ff6` |
| `tests/docs/19-threading.loft` | 2026-09-01 | `e9643ff6` |
| `tests/docs/20-logging.loft` | 2026-09-01 | `e9643ff6` |
| `tests/docs/22-time.loft` | 2026-09-01 | `e9643ff6` |
| `tests/docs/23-safety.loft` | 2026-09-01 | `b6dc9a61` |
| `tests/docs/24-json.loft` | 2026-09-01 | `f21577f2` |
| `tests/docs/25-generics.loft` | 2026-09-01 | `9764a37c` |
| `tests/docs/26-closures.loft` | 2026-09-01 | `64808d31` |
| `tests/docs/27-coroutines.loft` | 2026-09-01 | `320949cb` |
| `tests/docs/28-tuples.loft` | 2026-09-01 | `320949cb` |

## See also

- [RELEASE.md](RELEASE.md) § the per-release checklist — where this pass is reported
- [LIBRARY_DOC_REVIEW.md](LIBRARY_DOC_REVIEW.md) — the same watermark idea over the
  libraries, and the pass this one is modelled on
- [DOC_QUALITY.md](DOC_QUALITY.md) — how the prose should read once it is true
