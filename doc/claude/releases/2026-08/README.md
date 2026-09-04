<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# `2026-08` — release state (prep done 2026-08-01)

> The record of ONE release cycle — its blockers, the evidence each gate produced, and the
> decisions taken.  The process every cycle follows lives in
> [RELEASE.md](../../RELEASE.md); the index of cycles in [releases/README.md](../README.md).

The gate is the same: stability, not features. The `2026-08` **theme** work
(@PLN24 `#c`, @PLN23 database clients, @PLN4 HTTP server) is **not** in this
release — what ships is the heap-correctness body of work that accumulated over
the cycle, and the release is gated on that being clean rather than on the theme
landing. The theme carries to the next cycle.

**Tracker state.** `loft-lang/loft` has 7 open issues (#717–#723) and every one
is `fixed-pending-merge` — fixed on the cycle branch, closing automatically on
merge via their `Fixes #N` trailers. So the tag is taken against a **zero open
bug** tracker, but only *after* the merge: `main` on its own still carries four
`sev:high` SIGSEGV / miscompile faults, and is not releasable.

**Validation — each issue against its own reproducer**, on the interpreter, the
interpreter under `LOFT_POISON=1`, and `--native`:

| issue | shape | result |
|---|---|---|
| #717 | unreproduced SIGSEGV + crash report lost to a pipe | both guards green (`tests/scripts/717-closure-struct-return.loft`, `tests/crash_report_file.rs`) |
| #718 | `#remove` on an `index<T[..]>` owning a `text` | survived 2 |
| #719 | struct declaring both `sorted` and `index` over one type | survived 19 |
| #720 | `spatial<T[x,y]>` point subscript, all three roles | survived 9 |
| #721 | closure struct result used inline leaks its buffer | no leak |
| #722 | element bound out of a returned temporary | fields intact after churn |
| #723 | the same bind inside a loop | 1500 |

`fixed-pending-merge` is applied by automation off a commit trailer, so it is a
claim, not evidence — hence the table. #721 was additionally checked against a
control built at the revert commit (`02b50662`), which warns `1 stores not freed
at program exit` on the issue's own reproducer where this tree is silent.

**Gate evidence** (deliberate runs, per § The nightlies): `make ci` ALL GATES
PASSED — 3649/3649, one flaky (`keyframes_survive_total_datagram_loss`, a UDP
timing test that failed under the fully-parallel run and passed on retry; 12/12
in isolation, unrelated to this work). `LOFT_POISON=1` gate 1753/1753. `fmt` +
`clippy` clean. Found and fixed en route: a stale `index/target_surface.json`
(a builtin added without regenerating it — the branch-gates workflow does not
run that check, so only the full local gate catches it).

**Still owner-only and manual** (§ No Automated Releases): the merge, the tag
push, the draft build, validation and publish. Prep here is the version bump to
`2026.8.0` and the CHANGELOG roll-up, nothing further.
