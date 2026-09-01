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
| `tests/docs/01-keywords.loft` | 2026-08-31 | `8f851965` |
| `tests/docs/02-text.loft` | 2026-08-31 | `3e2555fc` |
| `tests/docs/03-integer.loft` | 2026-08-31 | `0c842212` |
| `tests/docs/04-boolean.loft` | 2026-08-31 | `16c7eb77` |
| `tests/docs/05-float.loft` | 2026-08-31 | `6e08b7d0` |
| `tests/docs/06-function.loft` | 2026-08-31 | `1b8c8fa7` |
| `tests/docs/07-vector.loft` | 2026-08-31 | `9fc947a5` |
| `tests/docs/08-struct.loft` | 2026-09-01 | `088676cb` |
| `tests/docs/09-enum.loft` | 2026-09-01 | `48c808b7` |
| `tests/docs/10-sorted.loft` | 2026-08-31 | `bb334f95` |
| `tests/docs/11-index.loft` | 2026-08-31 | `270267b2` |
| `tests/docs/12-hash.loft` | 2026-09-01 | `7a8a6322` |
| `tests/docs/13-file.loft` | 2026-09-01 | `088676cb` |
| `tests/docs/15-lexer.loft` | 2026-09-01 | `23743006` |
| `tests/docs/16-parser.loft` | 2026-09-01 | `512839a6` |
| `tests/docs/17-libraries.loft` | 2026-09-01 | `361aded3` |
| `tests/docs/18-locks.loft` | 2026-09-01 | `d50be135` |
| `tests/docs/19-threading.loft` | 2026-09-01 | `72ffb31a` |

## See also

- [RELEASE.md](RELEASE.md) § the per-release checklist — where this pass is reported
- [LIBRARY_DOC_REVIEW.md](LIBRARY_DOC_REVIEW.md) — the same watermark idea over the
  libraries, and the pass this one is modelled on
- [DOC_QUALITY.md](DOC_QUALITY.md) — how the prose should read once it is true
