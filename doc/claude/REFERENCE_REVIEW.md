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

## See also

- [RELEASE.md](RELEASE.md) § the per-release checklist — where this pass is reported
- [LIBRARY_DOC_REVIEW.md](LIBRARY_DOC_REVIEW.md) — the same watermark idea over the
  libraries, and the pass this one is modelled on
- [DOC_QUALITY.md](DOC_QUALITY.md) — how the prose should read once it is true
