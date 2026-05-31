<\!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Plan-12 — stdlib drain

Part of [@PLAN12 library extraction](README.md).  Covers
**Phase 3.6** — scope hygiene + CVE-surface lever.  Moves
surfaces that are NOT language primitives out of
`default/*.loft` into purpose-named libraries.  The drain
does NOT shrink to zero; the non-drainable floor (operators,
base types, control flow, format strings, core collection
ops, bootstrap I/O) stays embedded in the loft binary and
is covered by [security.md](security.md)'s advisory channel
permanently.

---

### Phase 3.6 — stdlib drain

**Dual purpose** (re-articulated 2026-05-31, after the Phase 6.7
security-channel design surfaced the asymmetry between drainable
and embedded stdlib):

1. **Scope hygiene** — move surfaces that are NOT language
   primitives out of `default/*.loft` into purpose-named
   libraries (HTML escaping doesn't belong in the language
   core; image types don't either).
2. **CVE-surface lever** — every surface that leaves the
   embedded stdlib becomes patchable on the library release
   cadence instead of the binary release cadence.  Faster
   security fixes for drainable territory.  The non-drainable
   floor (operators / base types / control flow / format
   strings / core collection ops / minimum bootstrap I/O) stays
   in the binary permanently and is covered by Phase 6.7's
   advisory channel for `"package": "loft"` entries.

**The drain does NOT shrink to zero.**  Phase 6.7 covers the
permanent floor.  3.6 is "move what should never have been in
the floor"; it isn't a strategy to externalise the entire
stdlib.

Done so far: **`escape_html` → new `lib/html/` — DONE 2026-05-27**
(with its test migrated from `tests/scripts/106` to
`lib/html/tests/01-escape.loft`, now `use html;`); Image / Pixel
already live in `lib/imaging/src/` (Format stays in default — it's
file-related and `lib/imaging` depends on it at load time);
**`02_images.loft` → `02_files.loft` rename DONE 2026-05-28** —
`src/wasm.rs DEFAULT_FILES`, `src/gendoc.rs`, the test fixtures
(`tests/generated/default.rs`, `tests/lib/p145_repro.rs`), the
load-order block in `CLAUDE.md`, and current-state references in
STDLIB.md / COMPILER.md / DOC.md / NATIVE.md / LIFETIME.md /
INTERMEDIATE.md / WASM.md / DEVELOPMENT.md updated; `path_sep()`
already lived there.

**Remaining (active):**

- Move `dir`/`basename`/`join(text,text)`/`resolve` from
  `03_text.loft` → `02_files.loft` (load-order safe — they only
  use primitives defined in `01_code.loft`; needs an audit that
  no `02_files.loft` declaration is shadowed).
- Audit call sites for new `use html;` lines.
- Future candidates as they mature: regex, JSON, CSV, base64,
  date/time helpers — each becomes a library package the same
  way `lib/html` did.  Schedule by maturity, not by clock.

**STAYS in stdlib permanently** (covered by Phase 6.7 advisories,
NOT by 3.6 drain):
- Operators, base type definitions, control flow primitives.
- Format strings (the `{x}` / `{x:j}` interpolation surface is
  shipped language behaviour).
- Core collection ops (`push`, `len`, hash insert/remove).
- The `null` sentinel and `??` operator.
- The bootstrap I/O surface needed by `01_code.loft`.
- JSON `{x:j}` format specifier + `text as Foo` cast (these
  ARE language behaviour, not library API — pulling JSON out
  breaks both).

