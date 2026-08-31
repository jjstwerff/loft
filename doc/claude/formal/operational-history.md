# formal/operational-history.md — the deviation register for [operational.md](operational.md)

> **The rules are next door.**  [operational.md](operational.md) states what must always be true of the
> language; this file is its TIMELINE — every place the code was measured not to do it, when,
> what it cost, and what closed it.  The two are apart because a contract a reader has to skim
> past its own history stops being a contract they can skim.  The rules doc carries the CURRENT
> state (how many are open, and which); everything below is the record behind it.

OPEN: **3** (D-op-1/2, and D-op-5 opened 2026-08-25 — two spellings of a following
null-check still report, the sibling of a wrapper-list drift fixed the same day; the null-model keystone deviations D-op-null-1/2 both CLOSED 2026-07-10 by
keystone steps 2–3, and D-op-6 opened AND closed 2026-08-29 by the first `@FR-E-NullArg` walk.
Opened 2026-07-10 by the @PLN102 pre-freeze audit —
[the null-model keystone decision](../plans/102-stability-contract/keystone-null-model.md).)

⚠ **This register read `OPEN: 2` while D-op-6 was live, and no measurement could have moved
it** — the rule it violates was itself incomplete. `(E-NullArg)` named comparisons as the only
exception to contagion and never mentioned truthiness, so a `&&` that answered `null` looked
like the rule being OBEYED rather than a shipped decision (C73) being broken. An `OPEN: n` is
only as strong as the rules above it, not only as strong as its oracle.

### D-op-6 — CLOSED (2026-08-29, the `@FR-E-NullArg` walk): `&&`/`||` kept a null right operand
- Was: `&&` and `||` coerced a null LEFT operand to `false` and let a null RIGHT operand through
  unchanged — `true && null` answered `null` where C73 says `false`. The parser types the whole
  expression the non-null `Type::Boolean`, so the null flowed out of a position the type system
  had already promised could not hold one: `r: boolean = t && maybe()` compiled clean and
  `r == null` was `true`; the same value reached a `boolean` STRUCT FIELD and a
  `vector<boolean>` element, and `(t && maybe()) ?? true` discharged it to `true`. On the field
  the compiler even emitted `redundant-null-check`, *"'on' is 'not null', comparison is always
  false"*, beside a comparison that answered true.
- Cause, and why it is one hole and not two: the lowering is `a && b` → `if a { b } else
  { false }`, so the LEFT operand becomes the `if` CONDITION and the jump coerces it
  (`OpGotoFalse` tests `!= 1`) while the RIGHT operand becomes a branch VALUE that nothing
  coerces. `convert` does not close it — every OTHER nullable type reaching a boolean position
  picks up a real conversion (`integer?` gets `OpConvBoolFromInt`, whose `!= i64::MIN` is
  already 0/1), but `boolean?` → `boolean` shares a base type and converts to nothing at all.
  So the coercion had one home, the jump, and the right operand never reached it.
- Fixed in `Parser::boolean_operator` (`src/parser/vectors.rs`), the one site that knows both
  operands are truthiness positions: a nullable-boolean right operand is wrapped in
  `b == true`, C73's raw compare, which answers `false` for the 255 sentinel. Parser-side, so
  both backends inherit it from one IR change; short-circuit is unaffected (the wrap stays
  inside the branch, measured by a counting right operand). Guard
  `tests/scripts/a-boolean-operator-answers-a-definite-boolean.loft`, falsified at `48544f1e`
  on both backends.
- Also corrected by the same walk, in the rule rather than the code: `(E-NullArg)`'s ordering
  clause claimed `boolean`, which has no ordering operator at all, and `(E-Truthy)` did not
  exist.

### D-op-null-1 — CLOSED (2026-07-10, keystone step 2): float/single null comparison now uniform
- Was: `float`/`single` null (a NaN) made `null == null` **false** and ordering unordered, where
  integer/char null is reflexive and orders low — violating `(E-NullArg)`'s uniformity.
- Fixed at the single source: the `Op{Eq,Ne,Lt,Le}{Float,Single}` `#rust` bodies in
  `default/01_code.loft` (which drive BOTH the interpreter via `fill.rs` regen and native codegen)
  now treat a NaN operand as null definitely — `null == null` true, `!=` its exact complement,
  null orders at the low extreme — matching `(E-NullArg)` and the integer/char behavior. Verified
  both backends against the matrix; guard `tests/scripts/pln102-null-comparison-uniform.loft`. The
  conversion set (docs/tests on the old `x != x` NaN idiom → `== null`) was migrated in the same
  change.

### D-op-null-2 — CLOSED (2026-07-10, keystone step 3): collision sites report, no longer silent
- Was: an op whose true result is the reserved `i64::MIN` pattern (or an out-of-range shift/cast)
  silently masked, saturated, or nulled a real value — the silent-wrong class `(E-Null)` forbids.
- Fixed at the single `#rust` source (drives BOTH backends), mirroring `÷0` (report + null +
  continue):
  - **Shifts** (step 3a): `OpSLeftInt`/`OpSRightInt` report `ShiftOutOfRange` on an amount outside
    `[0, 64)` or a left shift landing on `i64::MIN` (`1 << 63`); null operands stay contagious.
  - **Casts** (step 3b): `OpCastIntFromFloat` reports `CastOutOfRange` on a float outside integer
    range (was: saturate to `i64::MAX`); `OpConvCharacterFromInt`/`OpCastCharacterFromInt` report on
    an invalid code point (was: silent NUL); `OpCastIntFromText` reports when a *valid* number parses
    to exactly `i64::MIN` (an unparseable text stays DN3-nullable → null, silently, unchanged). NaN
    floats and null integers stay contagious.
- Distinct from **C85** overflow of ordinary arithmetic, which is a decided edge and stays silent.
- Both backends; guards `tests/scripts/pln102-shift-collision-guard.loft` +
  `tests/scripts/pln102-cast-collision-guard.loft`; the conversion set was one assertion
  (`inf as integer` saturate → null in `02-floats.loft`).

### D-op-1 — there is no shared operational semantics; the interpreter is the spec
- **Violates:** the premise of this doc (a single evaluation relation both backends obey)
- **Where:** `src/state/` (the interpreter) is the de-facto *executable* definition;
  `src/generation/` (native) is a *separate* generator. The rules across this operational
  family — this file's scalar core plus [heap](heap.md) / [iteration](iteration.md) /
  [coroutines](coroutines.md) / [concurrency](concurrency.md) / [calls](calls.md) /
  [matching](matching.md) / [tuples](tuples.md) / [closures](closures.md), all written
  2026-07-04 and each at 0 own deviations — are a written contract the code is *supposed* to
  meet, but none is mechanically checked against either backend.
- **Effect:** correctness for native means "matches the interpreter on the tests we ran",
  not "obeys the semantics". As of 2026-07-04 the gap is **no longer missing rules** — the
  operational rules are now written for every core area (store alloc/read/write/copy/free,
  iteration + combinators, coroutines, `par`, calls, `match`, tuples, closures, text
  formatting, interfaces/generics — the last two added 2026-07-05). What remains is that those
  written rules are not enforced against a *single evaluation relation both backends share*:
  they GUIDE the differential oracle rather than mechanically defining agreement. Nothing is
  left "spec = the interpreter's code" now — only the differential-vs-definitional gap itself.
- **Status:** OPEN — **the oracle is BUILT and growing (@PLN89).** `tests/differential_oracle.rs`
  runs `tests/oracle/*.loft` on BOTH backends and asserts they AGREE on stdout
  (value/null), exit code (halt), **stderr (what the program SAID — warnings and the diagnostic
  a fault renders)**, and leak-freedom, with a positive control proving the detector
  fires.  **2026-08-21 — the stderr channel landed (loft#1056).**  It had been captured since
  the oracle was built and never compared, which is how the same failed `assert` came to print
  a loft diagnostic on `--interpret` and a Rust panic naming a generated temp file on
  `--native`, for as long as both existed.  Seven corpus programs write to that channel, so it
  is exercised rather than agreeing by having nothing in it; the leak line is filtered out
  because leaks are their own channel and the native binary prints one only under
  `LOFT_NATIVE_LEAK_CHECK`.  **2026-08-21 — the call-stack cap converged (loft#1058).**  It was
  the last halting fault rendered two ways, and folding it found more than a rendering: the two
  guards on one cap counted different things, so `rec(9999)` printed an answer on `--interpret`
  and halted on `--native`.  The interpreter tested a `call_depth` counter that never counted
  `main` and was left untouched when a coroutine truncated the stack; it now reads
  `call_stack.len()`, the same quantity `cr_call_push` tests and `stack_trace()` reports on both
  backends, and native checks BEFORE pushing as the interpreter does, so a refused call is not on
  the stack the diagnostic reports.  `32-stack-overflow-halt.loft` pins the boundary from both
  sides.  Two guards enforcing one cap is the shape that drifts, and it drifted in silence
  because the corpus had no program near the cap.  **2026-07-04 coverage push** — the corpus now spans the divergence-prone areas where the
  two backends use the most different mechanisms: coroutines/generators (native state machine vs
  interp suspend), collection combinators (map/filter/comprehension), parallel reductions (par
  dispatch vs sequential), text (Rust String vs interp store), keyed collections (hash/sorted walk
  order + storage), and tuples/recursion — plus the two graduated cross-backend bugs (10/11).  **2026-07-04 — the DRIVER-AGREEMENT scope addition landed**: well-typedness is one static
  judgment, so `--dump` (pure parse+typecheck) / `--interpret` / `--native` must agree on
  accept-vs-reject; `statically_rejected()` (empty-stdout guard so a runtime panic isn't mistaken
  for a static reject) makes the #433 class — interp accepts what native rejects at rustc — a
  first-class caught property.  The oracle now catches real divergences in practice — three
  found this cycle: **#495** (runtime-Join over-free, FIXED), **#500** (native E0308 on a
  nested-ncc optional-text return, FIXED), **#501** (`.map`/`.filter` on a vector literal
  receiver, FILED).  Corpus is **32 programs** spanning coroutines / collections / parallel /
  text / keyed collections / tuples / nullability / nested enums / recursion / closures + the
  graduated bugs.  **NIGHTLY CI GATE WIRED (ci.yml, commit `971150dd`)**: the full
  `--ignored` sweep runs on the 03:00 UTC schedule + push-to-main (Linux-only, never on a PR),
  failing the nightly on any cross-backend divergence — the manual `-- --ignored` run is now
  a standing automatic guard.  Stays OPEN (the deviation closes only when a shared executable
  semantics replaces "the interpreter is the spec", or is reconciled): the corpus keeps growing.
  **2026-08-23 — the corpus can now FAIL, not only DISAGREE.**  The oracle asserts that the
  backends agree, and two backends wrong in the same way satisfy that perfectly — measured
  three times this cycle (the tuple-`&` local, the Join-arm ownership, the JSON walker),
  each identical on both sides and each structurally invisible here.  Re-measured, **32 of
  33 corpus programs carried no `assert` at all**: they printed, and the harness compared
  the two prints.  The hand-computed expected values existed — as COMMENTS (`// 55`,
  `// 3 2`, `// 0+1+1+2+3+5+8+13 = 33`), roughly 157 of them, checked by nobody.  They are
  assertions now (153 of them, 31 of 33 programs; a statically-rejected program is exempt
  because it never runs).  Converting a comment rather than recording a golden is what
  keeps the expectation independent of the implementation — a golden captured today would
  enshrine today's answers, shared mistakes included.
  Three ratchets keep the corpus from regrowing the hole, each proven able to fire:
  a program must produce OUTPUT (`@ORACLE_STATIC_REJECT:` to opt out), must EXIT 0
  (`@ORACLE_HALTS:`), and must carry an `assert`.  The first two are what make the third
  worth writing: a self-check that fails identically on both backends is otherwise just
  more agreement.
- **Removal:** build a **differential oracle** — run a growing program corpus on BOTH
  backends and assert they AGREE (value / null / halt / stdout / stderr / leak); these rules stay the
  written contract that GUIDES the corpus (what behaviour to cover), not a third
  implementation. A mismatch is then a divergence caught before ship, and every fixed
  divergence grows the corpus. *Chosen for now over an executable shared semantics (both
  backends conforming to one definition) — switchable to that later; these rules are reused
  either way.*

### D-op-2 — interp/native divergences are test-caught, not definition-caught
- **Violates:** E-Op / E-Uncomp / the shared-contract premise
- **Where:** the two backends are kept in agreement by the suite, so a divergence ships
  until a test happens to exercise it. **#433** is the canonical case: a program the
  interpreter evaluated fine failed to *compile* natively (`E0308`), i.e. the backends
  disagreed on a program both should accept — caught by a test, not by the definition.
- **Effect:** every codegen fix this session (the bool-arg E0308, the `__native_tail_ret`
  lift) was a backend disagreeing with the interpreter; under a shared semantics each is a
  definitional error, found before shipping.
- **Status:** OPEN — downstream of D-op-1 (the differential oracle).  The oracle now covers
  BOTH facets of "backends disagree": the run-both-and-compare (value/halt/leak) for a program
  both ACCEPT, and — as of 2026-07-04 — the accept-vs-reject *driver-agreement* for the #433
  facet itself (interp accepts a program native rejects at rustc). Closes with D-op-1.
- **Removal:** the differential oracle (D-op-1) makes "interp and native disagree on a
  program both accept" a *caught* failure (run-both-and-compare), not a coverage lottery —
  the corpus, not luck, decides what is exercised.
- ⚠ **What the differential oracle structurally CANNOT catch** (2026-08-25): a defect in the
  code the two backends SHARE.  Both consume the same parser IR, so a bad parser rewrite makes
  them agree — wrongly — and agreement is the oracle's pass condition.  Worked example: the
  defended-fault-site pass carried its own stale copy of the wrapper-getter list, so a defended
  OOB read of a non-integer vector reported while `vector<integer>` stayed silent — measured
  identical on interp and native, before AND after the fix.  So D-op-1 closing would not have
  found this, and the shared front end needs its own oracle rather than a differential one.
  Related: the reference-route discipline in [DEBUG.md](../DEBUG.md).

### D-op-5 — OPEN (2026-08-25): two spellings of a following null-check still report

- **Violates:** `(E-Report)` — *"a GUARDED site (the operand of `??` / **a following
  null-check**) emits the silent `*Nullable` op and reports NOTHING (the guard owns the null)"*.
- **Where:** `rewrite_defended_fault_sites` (`parser/control.rs`) recognises a guard as
  *a following sibling `if` that tests the variable*.  The rule says "a following null-check",
  which is wider.  Measured over six spellings of the same defended read, with a logger
  attached, on both backends:

  | spelling | reports? |
  |---|---|
  | `x = v[i]; if x == null` | no ✓ |
  | `x = v[i]; if x != null` | no ✓ |
  | `x = v[i]; if x == null \|\| …` | no ✓ |
  | `x = v[i]; g = 1; if x == null` | no ✓ — **was yes; fixed 2026-08-25** |
  | `if v[i] == null` (no binding) | **yes** ✗ |
  | `x = v[i]; match x { null => … }` | **yes** ✗ |

- **Effect:** a correctly defended program emits a runtime warning it did not earn.  The value
  is right in every cell — this is a REPORT-channel deviation, which is why a value-scored
  probe of this matrix comes back clean.
- **Why the two survive, and they are not one problem:** the no-binding form puts the fault
  site INSIDE the test rather than before it, so there is no `Set` to rewrite and the pass has
  nothing to key on; the `match` form has no `Value::Match` in the IR at all — it lowers to a
  subject temp, so the guard reaches the value through a COPY and the check names the temp,
  not the variable.  Closing the second means following a copy chain, i.e. dataflow, not
  adjacency.
- **Status:** OPEN.  Deliberately not closed by loosening the predicate: widening "guarded"
  SUPPRESSES a diagnostic, so an over-approximation goes silent on real faults while an
  under-approximation is merely noisy.  The 2026-08-25 widening was taken only as far as a
  soundness condition allows — *nothing else touches the variable between the site and the
  check* — with three negative cells (`a_null_that_escapes_before_its_check_still_reports`)
  pinning the loud direction.
- **Guards:** `tests/runtime_logging.rs` — `a_defended_fault_site_reports_for_no_element_type`,
  `a_null_check_after_an_unrelated_statement_still_owns_the_null`, and the two control cells.

> **D-op-4 — CLOSED (formalize4), so it is deleted from the list above.** The runtime no
> longer traps/halts on an uncomputable: div/mod-by-zero and integer overflow yield the null
> sentinel and continue on BOTH backends (E-Uncomp + E-Report), OOB already complied, and
> `NullDereference` was never raised. Guard: `tests/scripts/184-i333-div-zero-null-continues.loft`.
> The `??` trap-suppression mode is gone behaviourally, but **the `*Nullable` op split is NOT
> dead code** — an earlier version of this line called it "a separable cleanup", and measuring
> it (2026-08-25) says otherwise. The peers no longer differ in VALUE (both answer the null
> sentinel and continue), which is what that reading saw; they differ on the REPORT channel,
> which is E-Report's half of C80. The peers never call `s.raise`, so a swapped site is silent
> while an unswapped one logs — that is exactly what distinguishes a fault the program
> DEFENDED (`v[i] ?? fb`, or `x = v[i]; if x == null {…}`) from one it did not. Deleting the
> split would make every defended site report. Measured: of three sites in one program, only
> the undefended one logs, on both backends. Kept as a one-line tombstone because it reshaped two rules
> (E-Report's logging policy + the C80 refinement); see `git log` for the full entry.
