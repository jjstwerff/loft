<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# Contributing to loft

## Reporting a bug

Open a **[GitHub Issue](https://github.com/loft-lang/loft/issues/new/choose)**
using the **Bug report** template. The single most useful thing is a **minimal
reproducer** — the smallest `.loft` program that triggers it (try it in the
[web playground](https://loft-lang.org/loft/playground.html) first).

Please say:
- **expected vs actual**, and **which mode** (`--interpret` / `--native` / browser);
- a **workaround** if you found one — *and whether you actually ran it*. A
  workaround that doesn't work is worse than none; if you're not sure, say so.

**Our promise: your working program keeps working.** If an upgrade broke
something that used to work, that is a **top-priority regression**, not a
"managed change" or a "known limitation" — say so plainly and we treat it as a
bug (see [COMPATIBILITY.md](doc/claude/COMPATIBILITY.md)). And **which repo:**
the language or compiler → here (`loft/loft`); a *library's* behaviour → that
library's `loft-libs-*` repo; a game → its own repo. When in doubt, file here
and we'll route it.

## Known issues + the labels

Browse **[open Issues](https://github.com/loft-lang/loft/issues)**.  Triage uses a
small label vocabulary — what `sev:`, `wa:`, and `area:codegen` etc. mean is in
**[`.github/LABELS.md`](.github/LABELS.md)**, written so you don't need to read the
loft source. The most useful filters:
- `wa:none` — bugs with **no workaround** (you're blocked if you hit one);
- `area:<x>` — the subsystem; `sev:<x>` — severity.

## Contributing code

- Work on a feature branch; reach `main` via a pull request (never commit to
  `main` directly).
- The local gate before pushing: **`make ci`** (fmt → clippy → test).
- A bug fix should add a **regression test** (`tests/scripts/*.loft` for
  cross-backend behaviour) and reference the issue in the commit — `Fixes #NNN`
  auto-closes it.
- Deeper architecture + conventions live under [`doc/claude/`](doc/claude); how
  bugs/issues are tracked is [`doc/claude/ISSUE_TRACKING.md`](doc/claude/ISSUE_TRACKING.md).

loft is developed by building real consumers (the games **moros** / **dryopea** on
the **lavition** engine) and harvesting the language lessons — so bug reports
*from real programs* are especially valuable.
