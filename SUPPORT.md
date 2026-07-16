<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# Getting help with loft

## Found a bug?

Open a **[bug report](https://github.com/loft-lang/loft/issues/new/choose)**
using the **Bug report** template. The single most useful thing is a **minimal
reproducer** — the smallest `.loft` program that triggers it. Try it in the
[web playground](https://loft-lang.org/loft/playground.html) first; a link to a
failing playground snippet is a perfect repro.

**Our promise: your working program keeps working.** If an upgrade broke
something that used to work, that is a **top-priority regression**, not a
"managed change" or a "known limitation" — say so plainly and we treat it as a
bug. See [COMPATIBILITY.md](doc/claude/COMPATIBILITY.md).

**Every report gets a response.** A report is acknowledged and labelled — it is
never left to vanish, and a regression is never closed "wontfix". (For
maintainers, the process that turns a report into a fix is
[ISSUE_TRACKING.md § The public intake bridge](doc/claude/ISSUE_TRACKING.md#the-public-intake-bridge-arc-d).)

## Which repo?

- The **language / compiler / interpreter / runtime** → here (`loft/loft`).
- A **library's** behaviour (`web`, `graphics`, `time`, …) → that library's
  `loft-libs-*` repo.
- A **game** (moros, dryopea, …) → its own repo.

When in doubt, file here and we'll route it.

## Questions, not bugs?

The [documentation](https://loft-lang.org/loft/) covers the language and its
standard library. To read what the labels on an issue mean without digging into
the source, see [.github/LABELS.md](.github/LABELS.md).
