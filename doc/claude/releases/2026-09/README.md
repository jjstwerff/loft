<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# `2026-09` — release state (prep done 2026-09-04)

> The record of ONE release cycle — its blockers, the evidence each gate produced, and the
> decisions taken.  The process every cycle follows lives in
> [RELEASE.md](../../RELEASE.md); the index of cycles in [releases/README.md](../README.md).

The gate is the same: stability, not features. What ships is the store-lifetime and
reference-correctness body of work of the cycle — the two checkouts' streams joined on
`pr/join-2026-09-02` — and the release is gated on that being clean.

**Tracker state.** 22 open issues, **20 `fixed-pending-merge`** — fixed on the branch,
closing on merge via their `Fixes #N` trailers. The two without a fix were both filed by the
release pass itself and are routed: #1356 (a generator yielding a struct from its loop body
is refused on `--native`; the refusal names the workaround, and the one remaining
`SCRIPTS_NATIVE_SKIP` entry cites it) and #1357 (below). `#1342`'s library half — 34
undocumented types across six `loft-libs-*` repos — is done on their `doc-types-2026-09`
branches and lands with their next releases.

**Gate evidence on the joined tree** (deliberate runs, per § The nightlies):

| gate | result |
|---|---|
| `make ci` | ALL GATES PASSED, 4688/4688, 35 skipped (at `4c9a950e`; re-run after the doc commits below) |
| `M-leaks` | `LOFT_STORES=warn` over 1121 scripts: 0 leaking; `LOFT_LOG=stores` on the two threading scripts: nothing unfreed |
| `M-wasm` | `make wasm-html-test` 32 passed; `make gallery` built and every asset probed |
| `M-libs` | `revalidate_libs_local.sh`: 42 pass, 0 compile-break |
| `M-valgrind` | **RED** — 1157 interpreter + 36 native runs, **0 invalid accesses**, 11 files with definitely-lost blocks |

**The valgrind red is characterised, not waved through — #1357.** Every loss is a Rust
`String` grown by `append_text`/`format_text` and never freed, 8–88 bytes per file, all on
the interpreter, all clean under the store-leak gate. The text ledger (`LOFT_TEXT_TIMELINE`)
attributes nine to the class #568 closed in July — so that fix does not reach these shapes —
and cannot see the other two (a `par` worker, a `format_float` tail). Two facts frame the
call: the pre-join sweep the same morning had 23 such files, so the day's fixes removed
twelve rather than adding any; and `M-valgrind` has never carried an `[x]` in any release —
this is the first cycle it was measured rather than left unticked. Also found: CI's LSan
gate suppresses exactly this class, on a premise `scripts/valgrind.supp` records as measured
false. **Whether a fixed few bytes at exit, with no invalid access, holds the tag against
§ Safety gate's "definitely lost: 0 bytes" is the owner's decision**, and the issue carries
everything needed to make it.

**The reference is reviewed at its current source, 40 chapters of 40**, and the pass found
five caveats promising limitations the language no longer has — each cited an issue that had
since closed, each kept its sentence when its tag was stripped. Settled by running every
claim on both backends (REFERENCE_REVIEW.md, above the watermark table). Documentation
review steps 1–4 and 8 done; `doc_history_report` was run and the docs at its head stay,
for these reasons: `PAR_PRESENTATION` (24%) is a presentation, not a contract doc;
`USER_FACING` (19%) is a census whose dated table IS its state; `formal/closures.md` (17%)
already has its companion and what remains flagged are live pointers and the `OPEN: 0`
header; `formal/matching.md` (13%) carries the @PLN35 "SHIPPED" provenance block, a
companion move deferred to a doc pass rather than done under a release; `WINDOWS.md` (11%)
likewise.

**`M-ignores` evidence for the owner's sign-off.** One `#[ignore]` (`regen_fill_rs`, a
maintenance regenerator). Skip lists: `447-coroutine-yield-borrow.loft` (→ #1356, the
refusal stands), `web/http.loft` twice (live HTTPS, not a gap), and `input` REMOVED — its
blocker #248 closed in June and the package now lives in loft-libs-game, so the entry
matched nothing.

**Still owner-only and manual** (§ No Automated Releases): the merge via PR — after which
`release-gate.yml` is on the default branch and `make release-gate` can run — the tag push,
the draft build, validation and publish, and the #1357 call above.
