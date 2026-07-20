<!-- Copyright (c) 2026 Jurjen Stellingwerff -->
<!-- SPDX-License-Identifier: LGPL-3.0-or-later -->

# @PLN102 arc-E flip-gate — coverage-gap analysis (the two de-risking studies)

> **Update (2026-07-20): both HIGH items BUILT.** Finding-1's gate = `tests/e1_code_set.rs`
> (pins the 4 codes, 2 teeth, positive-control verified). Finding-2's HIGH rows = the
> behaviour corpus grew 22→47 lines (rendering, structs, struct-enum payload, keyed
> collections, text stdlib, null keystone, math, sort) — interpret==native verified.
> Building it surfaced a real bug: `h[absent] ?? <mismatched-type>` (e.g. `Row? ?? int`)
> is not rejected by loft's type checker → the interpreter **SIGSEGVs** while native gives
> rustc `E0308` (a silent-crash + backend-divergence bug); omitted from the corpus. Fix design + boundary matrix: qq-type-mismatch-fix.md (still TO FILE as an issue).
>
> **Status: ANALYSIS (2026-07-19).** Answers the two falsification questions
> [flip-gate.md](flip-gate.md) left open before the `0→1` flip: **(1)** is there a
> FOURTH mechanically-driftable frozen surface with no gate? and **(2)** is the Gate-2
> behavioural corpus complete enough to catch a silent semantics break? Both are
> irreversible-miss risks (the flip is a one-way door). Findings + a concrete worklist
> below; nothing is a hard blocker, but the **HIGH** items should land before the flip.

## Finding 1 — the fourth ungated frozen surface: the E1 diagnostic CODE SET

**The gap.** E1 declares the diagnostic **code** (a kebab-slug, rendered
`error[shift-amount-out-of-range]:`) the *frozen machine handle* — prose stays
improvable, the code is the stable contract. Four codes exist today:

| Code | Emitted at |
|---|---|
| `cast-constant-out-of-range` | `src/parser/operators.rs` (E2-B) |
| `shift-amount-out-of-range` | `src/parser/operators.rs` (E2-B) |
| `text-parse-may-fail` | `src/parser/operators.rs` |
| `format-unescaped-brace` | `src/lexer.rs` (×3 sites, E2-A) |

**No test pins the code set.** Worse, the `code!` harness *actively strips* the tag
(`tests/testing.rs::strip_diag_code`, added in E1 so prose assertions stay code-agnostic),
and no `tests/error_messages/baseline/*.expect` exercises a coded diagnostic. So renaming
`shift-amount-out-of-range`, removing a code, or silently swapping which site emits which
code breaks **zero tests** — yet each is a break of the frozen machine handle. This is a
genuine fourth ungated surface (LAYOUT/BEHAVIOUR/API-shape are gated; this is not).

**Recommended gate (small, inert-friendly):** a golden that enumerates the code set — one
minimal program per code that asserts the bracketed slug under `LOFT_ERRORS=compact` (the
harness must NOT strip it for this test), plus a committed `codes.txt` listing all codes
so an add is a reviewed diff and a rename/removal is red. Ties into the flip-gate the same
way as the other goldens (a code change ⇒ a contract decision). **Effort: S.**

**Two secondary thin spots** (the surface HAS a gate, coverage is just thin — a corpus
line, not a new gate):
- **Bare `null` + `char`/`byte` value→text rendering.** The format-spec grammar is gated
  by `tests/scripts/63-format-edge-cases.loft` / `67-format-align.loft` (both backends),
  but these two renderings are untested. → corpus line (see Finding 2 HIGH-render).
- **Binary file-I/O default per-type byte encoding.** Round-trip + cross-backend parity
  only (`tests/binary_io_matrix.rs`, `#[ignore]` by default) — NOT a wire-byte golden. The
  struct-layout half IS caught by F9; a coordinated writer+reader scalar-encoding change
  would pass round-trip yet break previously-written files. → decide if persisted binary
  files are in never-break scope; if so, a small committed-bytes golden. **Effort: S–M.**

**Everything else surveyed is gated or non-contract:** JSON *parser* gated
(`tests/json_corpus.rs` byte-golden); JSON *writer* is CLI-tooling only (no loft builtin
emits it); **CBOR does not exist** (only a doc comment); store/layout gated (F9 +
`store_durable_format.rs`); api-surface + exit codes gated; `introspect` / `--show-ownership`
/ `gendoc` are deliberately non-contract dev tools. The error *boundary* (which programs
error) is gated **by example** in both directions (`error_messages.rs` byte-golden with
exit codes, `parse_errors.rs`, the behaviour corpus) — as wide as the corpora, no more.

## Finding 2 — the behavioural corpus is honest but THIN

`tests/golden/behavior/corpus.loft` covers the arithmetic / null / cast / shift / vector /
match / text-slice **spine** (23 lines) well, but omits whole load-bearing pillars. **None
of the gaps are untested** — the ~330 `tests/scripts/*.loft` run on BOTH backends
(interpreter via `wrap.rs::loft_suite`, native via `native.rs:833`) and assert values,
some via rendered-string compares. But that suite is **not contract-versioned**: a
post-flip silent break there is a red test someone can re-bless without the
`CONTRACT_VERSION`-bump ceremony. The corpus is the *one* frozen, contract-versioned
stdout artifact, so the value is **promoting** load-bearing surfaces into it — especially
**rendering**, where the assert suite is weakest and silent drift is most notorious.

### The gap worklist (each a deterministic, backend-stable labelled line)

**HIGH — load-bearing + high silent-break risk (land before the flip):**
- **Number→text rendering** (the single best fit — assert-weakest, drift-prone):
  `{1.0/3.0}` (float default precision), `{-42:03}` (sign+pad), `{null_int}` → `null`,
  `{255:#x}` (radix+prefix), `{[1,2,3]}` (vector render), `single` vs `float`, large i64.
- **Structs** — construct + nested field read + field write-back.
- **Struct-enums (payload variants)** — construct + `match` destructure + `is` capture
  (a distinct discriminant+field codegen path; JsonValue rides on it).
- **Keyed collections** — hash insert/lookup/remove(→null), sorted/index iteration ORDER,
  one range-count (the DATABASE.md core: key-compare / Morton-order / OpHashRemove).
- **Text stdlib beyond len/find/slice** — split/join, replace, upper/lower, contains,
  `size()` (bytes) vs `len()` (chars, the fresh @PLN110 split).
- **Null-model edges** — `{null}` render, `x == null`, null PROPAGATION (`(null+3)??-1`),
  `!null` (the @PLN102 keystone semantics this flip exists to freeze).
- **Math stdlib** — abs, clamp, sqrt, `**` (right-assoc, `2**3**2`=512), `floor_mod`
  (negative-operand sign rule).
- **Sorting + aggregates** — `sort()` (text+numeric), `min_of`/`max_of` (empty→null).

**MED — real but narrower / better-covered elsewhere:**
closures WITH capture · references/links + `&`-writeback · coroutines/custom iterators
(`yield`, `next`→null) · tuples (`.0`/`.1`, destructure, mixed-width) · integer narrowing
SUCCESS + widening + sign-extension (corpus only has the failing `300 as u8`) · character
ops (`'a' as integer`, char match, `#index`/`#next` offsets) · control-flow rendering
(if-expr value, while, for-range, `?? return`) · JSON (@PLN109 exact-integer
`9007199254740993`) · `par()` deterministic fold · generics/interfaces/dynamic dispatch.

**LOW — skip / defer** (environmentally awkward or already matrix-tested): file I/O +
FileResult, seeded random (brittle snapshot), value-struct custom operators, three-state
boolean.

## How this feeds the flip gate

- **Finding 1** adds one item to the flip preconditions: **build the E1-code-set gate**
  (S) — the fourth surface, currently silent. The 2 secondary spots fold into the corpus
  work / a scoped binary-format decision.
- **Finding 2** turns "the corpus is a starting set" into a **concrete pre-flip worklist**:
  land the **HIGH** rows (the rendering block first) before the flip; MED/LOW can grow the
  corpus post-flip additively (adding a corpus line is not a contract change).
- Neither is a *hard* blocker — the assert suite already tests these behaviours on both
  backends — but both close the "silent break slips through the ONE frozen artifact" risk
  that the flip's one-way door makes permanent.

## See also
- [flip-gate.md](flip-gate.md) — the gate + drift gates + the falsification bullets this answers.
- `tests/golden/behavior/corpus.loft` · `tests/behavior_golden.rs` — the Gate-2 instrument to extend.
- `src/diagnostics.rs` (`DiagEntry.code`) · `tests/testing.rs::strip_diag_code` — the E1 code surface + the strip that hides it.
