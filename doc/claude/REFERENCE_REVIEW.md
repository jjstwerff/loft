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
not re-verify them by hand; that budget belongs to the things no test covers:

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

## See also

- [RELEASE.md](RELEASE.md) § the per-release checklist — where this pass is reported
- [LIBRARY_DOC_REVIEW.md](LIBRARY_DOC_REVIEW.md) — the same watermark idea over the
  libraries, and the pass this one is modelled on
- [DOC_QUALITY.md](DOC_QUALITY.md) — how the prose should read once it is true
