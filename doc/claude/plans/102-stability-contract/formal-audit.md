<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN102 — the language-basis + error pre-freeze audit (the worklist)

> **Status: prep drafted 2026-07-10.** The **language-side** companion to
> [lib-audit.md](lib-audit.md): a freeze-audit of loft's **formal spec** (`doc/claude/formal/*`,
> the language's basis) and its **error surface**. Both freeze forever at contract 1
> ([COMPATIBILITY.md § Before the flip](../../COMPATIBILITY.md)). Sourced from a 6-agent survey
> (4 over the formal docs, 2 over the error surface), every runtime claim **verified on both
> backends**.
>
> **How to use it — same as [lib-audit.md](lib-audit.md#the-disposition--lean-toward-improving-not-toward-freezing-what-we-have):**
> each item is a design decision worked with its **alternatives presented** and its **conversion
> set enumerated**; lean toward *improving* (freezing an illogical rule imposes it on everyone
> forever). Two disposition twists specific to this half:
> - **The formal deviation register is already ~closed** — the code obeys the rules — so this is
>   NOT deviation-hunting. It is judging whether the **rules and decided edges are the right
>   permanent choices**, and pinning the **gaps** (unspecified behavior freezes as impl-defined).
> - **The error surface is one-directional** ([COMPATIBILITY.md § one-directional](../../COMPATIBILITY.md)):
>   post-freeze we can DROP an error but never ADD one, so the disposition for errors **inverts** —
>   be **maximally strict now**. The question is *"do we need **more** errors?"*, and every
>   too-permissive spot is a last-chance-to-add (conversion cost noted per item).

---

## THE KEYSTONE — the in-band null-sentinel model (one decision, every audit)

The single thread through the lib, formal, and error audits. `null` is an **in-band sentinel**
(`i64::MIN` for `integer`, `NaN` for `float`/`single`, `255` for `u8`/`bool`, codepoint-0 for
`character`, `nullref` for references). Freezing the model freezes its collisions — and **the
formal spec currently *denies* they exist**, which is itself a must-fix:

| Face | What freezes, silently | Verified / ref |
|---|---|---|
| **The spec claims the sentinel is unobservable** — `operational.md` E-Null: "how a backend encodes the sentinel is its business" | the model is sold as "a slot of `τ` never holds a non-`τ` value," but it is an in-band value with observable collisions; freezing E-Null locks an abstraction loft does not provide | E-Null; `types.md` prose |
| `null == null` is **true for integer/char, false for float** (NaN) | `x == null` silently means different things by type; even `x == x` differs | verified both backends; `fill.rs:636,1018` |
| `null` orders as **−∞ for integer** (`null < 5` → true) but **incomparable for float** | a nullable `sorted`/`index` key or a hand `if x < limit` treats missing as smallest, no signal | verified; `fill.rs:650` |
| the declared `integer` range **includes its own `i64::MIN` sentinel** (off-by-one) | `1<<63`, `abs(i64::MIN)`, `"-9223372036854775808" as integer` all collide → null | `data.rs:96` reserves `MIN+1`; verified |
| overflow (**C85**) writes the sentinel into a **non-null** `integer` slot | the non-null guarantee is advisory for arithmetic — a frozen soundness hole | `types.md` N-Arith vs C85 |
| the two formal docs cite **different** sentinel constants (`i32::MIN` vs `i64::MIN`) | the frozen constant is ambiguous; per-width sentinel steals a real value (`u8` null=255) | `types.md` table vs `operational.md` |

**Decide this first.** It is the @PLN102/@PLN25 boundary and it changes signatures
(`find`/`min_of` return types, `==`/ordering on null). **Alternatives:** (A) carry an explicit
nullness bit for `integer` — retire the sentinel; (B) keep the sentinel but *confront it in the
spec* — state it is in-band and observable, exclude it from the value range, give the collision
ops honest nullable types, and add the errors below. Conversion cost of (A) is high; of (B),
concentrated in the newly-added faults (near-zero, per the error audit). Either way the spec's
"encoding is private" claim must go.

---

## The must-fix set (High) — pre-freeze-only, in decision order

> **RECONCILED against `main` 2026-07-13 (verified live on both backends).** Most of this set has
> since landed — the table below is kept for the analysis, but the true open set is much smaller:
>
> | Item | Status now | Evidence |
> |---|---|---|
> | F1 float `==` | ✅ **exact** | `1.0 == 1.0000000001` → false; `!=` exact complement |
> | F2 compound-assign | ✅ **single-eval** | `v[idx()] += 5` calls `idx()` once |
> | F3 `&&`/`\|\|` short-circuit | ✅ **specified** | `operational.md` E-And/E-Or; E-Left scoped to non-short-circuit |
> | F5 match non-enum + guards | ✅ **works** (spec = confirm `matching.md` states it) | `match x { 2..=5 => … }` + `n if n>2` run |
> | F6 `&v` alias | ✅ **aliases** + sandbox hole closed | `w=&v; w[0]=99` → `v[0]==99` (reconcile `heap.md` "copies" prose) |
> | F8 comparison chaining | ✅ **rejected** (non-assoc) | `1 == 2 == 3` → "comparison operators do not chain" |
> | runtime errors | ✅ **stable kinds** | `RuntimeErrorKind::{CastOutOfRange,DivideByZero,ShiftOutOfRange}` |
> | layout guard wired | ✅ | `allocation.rs:2726` refuses a mismatched-layout load |
>
> **E1 — DISSOLVED (owner decision 2026-07-13).** *"The error texts are not frozen; we freeze WHEN we
> throw an error, and can only LIMIT the space we error for."* → Diagnostic **prose is not a frozen
> surface** — it stays freely improvable forever (the goldens are our own tests, they update with the
> prose). No stable codes are needed. What freezes is the **error BOUNDARY** (which programs error),
> and it is **one-directional: the error space can only shrink post-freeze** (stop erroring on X →
> additive; never start erroring on a program that used to compile). So E1 becomes a **one-line
> contract statement** in COMPATIBILITY.md, not 457 codes.
>
> **This makes E2 the pre-freeze headline:** be **maximally strict NOW** — every error we will ever
> want must land before the flip, because we can never add one after. See § The error surface.
>
> **Genuinely OPEN:** **E2** (the error-boundary maximal-strictness audit — *the* priority) · **F9**
> (guard is wired, but `layout_algo_hash` at `types.rs:1674` omits **endianness** + the
> **not-null↔nullable** distinction) · **F7** (`ref ==` is identity, not structural — a decision) ·
> **F4** (assign place/RHS eval order — re-verify) · spec-honesty for the ✅-behavior rows.

Several of these are **semantic changes, not just added errors** — they can *only* land while
contract 0 allows, because after the freeze changing an observed value is a regression:

| # | Item | Why pre-freeze-only | Verified/ref |
|---|---|---|---|
| K | the null-sentinel model (above) | changes signatures + values | — |
| F1 | **float `==`/`!=` are epsilon-approximate but `<`/`<=` exact** — `1.0 == 1.0000000001` → true; `a<b` **and** `a==b` both true near the boundary; `!=` not the exact complement of `==` | a trichotomy/transitivity violation frozen forever; no formal rule pins it at all | `fill.rs:1018,1025` |
| F2 | **compound assignment double-evaluates its place** — `w[f()] += g()` calls `f()` **twice** | a silent duplicate side-effect; lowering the place to eval-once is a semantics change | verified both backends |
| F3 | **`&&` / `||` short-circuit is unspecified** — and the one general rule (E-Left) says both operands evaluate (false) | the most-depended-on eval rule is unwritten; freezes as impl-defined | verified; `operational.md` E-Left |
| F4 | **assignment place-vs-RHS eval order unspecified** (`a[f()] = g()`); E-Asgn prose "RHS first" contradicts observed LHS-first | evaluation-order gap = the canonical silent-freeze trap | verified `[L][R]` |
| F5 | **`match` guards have no formal semantics**; **`match` on non-enum** (int/range/literal) unspecified | a shipped feature freezes as impl-defined | `matching.md` |
| F6 | **`&v` on a vector: `heap.md`+`capabilities.md` say COPIES; `binding.md`+reality say ALIASES** — and the sandbox soundness proof rests on the false "copies" premise | a memory-model spec contradiction + a possible sandbox-admission hole; must reconcile AND verify the `&`-alias-into-host write is gated | verified `v[0]==99` |
| F7 | **reference `==` defaults to identity, not structural** — unspecified, observable (a view equals its source, two field-equal structs don't) | frozen identity-vs-structural split with no rule | `01_code.loft:862`; `mod.rs:5429` |
| F8 | **comparison operators are one left-assoc level** — `a == b == c` type-checks and misbehaves on booleans | non-associative comparison is a grouping change (pre-freeze-only) | `grammar.md` level 3 |
| F9 | **layout persistence guard not wired into the load path** (D-layout-1); layout hash ignores **endianness** and can't see a **not-null↔nullable** schema flip | the frozen persistence promise's own detector doesn't run; a stale/foreign store reads raw | `@PLN97`; `types.rs:1674` |
| E1 | **diagnostics have no stable identity code** (`DiagEntry` = level+message) + 41 golden baselines → loft freezes **error prose as identity** | add a stable code/kind now → prose stays improvable forever behind a frozen code | `diagnostics.rs:16` |
| E2 | **missing errors (the too-permissive class)** — see the error section; the sentinel-collision adds are ~zero conversion cost | adding an error is the one-way door | verified |

---

## The error surface — "do we need more errors?" (the one-way door)

Post-freeze loft can only DROP errors, so **add every error we might want now.** The too-permissive
findings, with conversion cost (the trade-off you weigh):

### Tier 0 — soundness CRASHES (both FIXED 2026-07-13, `tuxedo-compat-gate-design`)
The E2 sweep found two SIGSEGVs (both backends) where C80 requires null/drop, not a crash. Neither
is a "missing error" — they are the interpreter using a bad value as a pointer. Both fixed:
- **default-arg type mismatch** (`fn f(x: text = 42)`) — an unchecked wrong-typed default reached
  runtime as a pointer. Fix (72bc5302): type-check + coerce the default against the param type at
  parse time, like a call-site arg; only a KNOWN-vs-KNOWN by-VALUE mismatch is rejected (27329716
  narrows: skip `null` / untyped-literal / by-reference-param defaults, which `convert` would
  mis-coerce). `text = 42` → clean error; `float = 5` → `5.0`.
- **null-struct field access** (`o: T? = null; o.field`) — read/write of a field of a null struct hit
  `store()`/`store_mut()` with the null `store_nr` (65535) → OOB `allocations` panic. Fix (91246f2d):
  every scalar field op guards `rec == 0` at the `#rust`-template chokepoint (feeds both backends) —
  read → the field type's null, write → drop (C80). Regression: `tests/scripts/558-*`.

**`field == null` on a non-nullable field — codegen FIXED (7bc01e1e).** The MIS-CODEGEN was
enum-specific: `s.c == null` for a value-enum field lowered to `OpRefIsNull(OpGetEnum(s,off))` — a
store_nr sentinel test on the enum DISC BYTE — so native emitted `.store_nr` on a `u8` (E0610) and
the interpreter returned the wrong value.  Root: the enum_null branch used OpRefIsNull for every
non-`__nullable` enum, and a value-enum field access is typed `Enum(_, true, _)` (the access-context
is-reference flag is polluted).  Now decides value-vs-reference from the enum DEFINITION's `returned`
type: a value enum tests the disc (`OpConvIntFromEnum == 0`), only a struct-enum keeps OpRefIsNull.
Correct on both backends (present → false, read through a null receiver → true, C80).  Text fields
were never mis-codegened (they use `OpEqText` + the text null) — the E0308 note conflated them.
**RESIDUAL — FIXED (c9fc8153).** The spurious "Redundant null check" / "Redundant null coalescing"
warning on `field == null` / `field ?? d` for a NULLABLE receiver: `field()` now clears
`expr_not_null` when the receiver's TYPE is `Optional` (a field of a nullable receiver reads null when
the receiver is absent, C80).  The signal is the receiver TYPE, not `expr_not_null` — a non-null
CONSTRUCTED struct (`g = P285G{…}`) leaves `expr_not_null` false too, so the earlier type-agnostic
attempt wrongly killed the genuine p285 warning.  Only the lint is cleared (the type is unchanged;
widening to `Optional` would force `?? d` on every nullable-receiver field read).  Nested
`w.inner.tag` propagates.  So `field == null` on a non-nullable field is now fully resolved.

### Tier 1 — RE-MEASURED 2026-07-13 (`tuxedo-compat-gate-design`)
Running the current tree flipped most of the table below: the runtime already reads NULL for
out-of-range operations (C80), so the "silently MASKED to a wrong value" concern is largely gone.
The only genuine silent-*wrong values* were two, both FIXED; the rest are already-resolved or
sentinel-collision edges the owner's C80 rule says accept.

| Item (re-measured) | Status |
|---|---|
| **cross-type `==`/`!=` → truthiness** (`5 == "banana"` → **true**) | **FIXED** (449c4f64) — reject at compile, like the ordering ops. See § below the K-table note. |
| **`single as integer` overflow → i64::MAX** (saturates; `float as integer` already nulled) | **FIXED** (b230b957) — mirrors float→int: overflow reads null (C80). |
| **`enum == int` → discriminant leak** (`Color.Green == 1` → **false**, comparing the +1-biased internal disc) | **FIXED** (b5d33b5d) — reject at compile, "No matching operator"; enum==enum / enum==null untouched. |
| `1e30 as integer` (float) → **null** | already C80-correct (bounds check → i64::MIN). No add. |
| `"…" as integer` (text parse) → **null** | already a COMPILE ERROR (`as integer` "may fail — use `integer?`"). No add. |
| `1 << 100` / `1 << -1` (out-of-range shift) → **null** | already C80-correct (`ShiftOutOfRange` → i64::MIN). No add. |
| `999999999 as character` → NUL | renders null; C80-adjacent. No genuine wrong value. |
| `sqrt(-1)` / `log(-1)` / `asin(2)` → **null** | **ACCEPT** ([C80](../../DESIGN_DECISIONS.md): undefined → null, never a fault). No decision needed. |

**Debatable — need the owner's call (do NOT unilaterally add; each conflicts with "null is a valid
result / no runtime errors"):**
- **Sentinel collisions** — a VALID value that equals the null sentinel reads as null: `1 << 63`,
  `abs(i64::MIN)`, and the (already compile-rejected) `"-9223372036854775808" as integer` all land
  on `i64::MIN`. Fix = reserve the sentinel (a representation change — `data.rs:96` already reserves
  `MIN+1` for the *declared range*, so the collision is narrow) OR accept. C80 leans accept.
- **Constant out-of-range as a COMPILE error** (`1 << 100`, `1e30 as integer` where the operand is a
  literal) — the runtime already nulls these (C80-correct), so a compile error would reject a program
  with well-defined (null) behaviour. Stricter, but arguably against the spreadsheet model.
**Nullable value-enum null-check — FIXED (9582a70a).** `n: Color? = null` rendered `null` but
`n == null` was false, `n ?? d` didn't coalesce, `if !n` read present.  Root at the PRODUCER:
`convert(Null, Enum)` skipped `OpConvEnumFromNull` (the `FromNull` loop matches by RETURN type, and
that op returns the generic `enumerate`, never `is_equal` to a specific `Enum(Color)`), so the slot
held a bare-null byte 0 while every consumer tests the 255 sentinel.  Fixed by converting a null
value-enum target to its typed null (255) — variables, reassignment, returns, inline struct fields,
both backends.  A `vector<Color?>` null literal stays rejected (`parse_vector` guard; the per-element
null slot is unwired).  Tests: scripts/560, parse_errors.

**Regression found + fixed while doing the above (b8a80870).** The cross-type-`==` guard (449c4f64)
refused `OpEqBool` for ALL non-boolean operands, but `DT? == DT?` (a nullable STRUCT-REF, two present
refs) legitimately uses `OpEqBool` as its null-ness test (no `OpEqRef` coercion exists for
`Optional(Reference)`) — the guard stranded it with "No matching operator".  Earlier suite runs missed
it on a STALE test binary.  Restricted the reject to VALUE operands (scalars + enums); references keep
their behaviour.  Lesson: a `touch src/*.rs` / clean rebuild before trusting a green `wrap loft_suite`.

### Semantics changes — must be pre-freeze (changing an observed value is a later regression)
| Sev | Item | Fix | Conv. cost |
|---|---|---|---|
| High | float `==` epsilon (F1) | exact `==` + named `approx(a,b,eps)`, or a warning | **high** (many float compares) — decide now |
| Med-High | text classifiers vacuously `true` on `""` (`"".is_numeric()`) | return `false` on empty | low but nonzero |
| High | `File.write()` returns void — failed write silently swallowed | return `boolean`/`FileResult` | ~0 for discarding callers |
| High | `content()`/`read_bytes()`/`list_dir()` → `""`/`[]` on missing file | additive checked/nullable variant | low-medium |

### Reconsider the spreadsheet model's quietness (C80/C85 — decided, but frozen)
- **integer overflow → null but the type stays non-null `integer`** (C85) — invisible at compile
  *and* silent at runtime, while div/parse are typed `integer?` (DN3). At minimum an opt-in
  overflow-site warning (costs nothing); forcing `integer?` is what C85 declined (very high cost).
- **div0/mod0/OOB/null-deref → null+continue, silent by default** (C80) — a bare `loft prog.loft`
  emits nothing (only div0 warns, and only for a *constant* divisor). Decide the runtime loudness.
- Each of these already has a DESIGN_DECISIONS home; the freeze just needs them stated *as* the
  frozen contract, not left implicit.

### Error-surface hygiene (identity, taxonomy, drops)
- **E1 (the headline):** add a stable diagnostic code/kind so prose stays improvable.
- The **`null(/0)` format-suffix** leaks fault identity into observable output — but for only 4
  fault kinds (inconsistent). Decide if it's frozen; if kept, cover all faults; else flip
  `LOFT_FORMAT_BARE_NULL` to default.
- **Recoverable-vs-halting fault boundary undocumented**; **halting faults mode-dependent** (halt
  in dev, continue in prod) — pin both as the contract.
- **Drop before freeze** (dropping stays available, but cleaner deliberate): dead
  `RuntimeErrorKind` variants (`NullDereference`, `NarrowCastOverflow`-if-not-wired), the dead
  `*Nullable` op split, the dead `not null` field-hint path.
- **Renderer vocab disagreement** (`Debug` vs `note`); compact format is an unversioned parseable
  line (add `--errors=json` so the human line stays improvable).
- **Unterminated-format-brace is a WARNING not an error** — if malformed interpolation should
  reject, promoting Warning→Error is a tightening only contract-0 allows (last chance).

---

## Per-area formal findings

### Types / null / binding / tuples
- The null-sentinel cluster (keystone). Plus **decided edges to state honestly**: C85 overflow-non-null
  (fix the `types.md` prose that denies it), DN3-vs-C85 asymmetry (`/` nullable, `*` not), B-View
  struct-projection-aliases, DN6 null-join excludes `text` (impl leak → make uniform or accept).
- **Gaps (freeze impl-defined):** **tuple/struct equality entirely unspecified** (structural? does it
  inherit float epsilon + null?); **text comparison unspecified** (byte vs codepoint, case, null-vs-empty);
  double-optional collapse `τ?? ≡ τ?` erases which layer was null; tuple-of-reference / tuple-null
  unspecified; no unit/1-tuple type; `char`-null (`'\0'`) collides with a real NUL, both invisible.

### Ownership / heap / layout (the memory model — highest stakes)
- **F6 (`&v` copies-vs-aliases contradiction)** and **F7 (reference `==` identity)** above.
- **C80 write-side drop:** `obj.field = x` when `obj` is null **silently discards the write** and
  continues (`H-WriteNull`/`H-WriteOOB`) — the write side is more dangerous than the read side and
  isn't separately justified. State it deliberately.
- **F9 persistence** + **gaps to pin:** the on-disk `text`/string record format is unspecified; the
  frozen format caps (65536 stores, 256 enum variants, u32 rec/pos) aren't stated; the single-arena
  invariant (`L-Ref` = 4-byte rec, no store_nr) is implicit; `L-Struct` field-packing tie-break
  (declaration order) and `L-Narrow` range→width function are examples-not-rules.
- `H-FreeLIFO` freezes a LIFO allocator as a *hard fault* — an internal constraint the compat
  promise doesn't require; keep it outside the frozen contract or accept it deliberately.
- ownership.md's O-Deps "0-open/formal" headline oversells vs the honest-floor body (validation, not
  proof; runtime Join witness) — soften the headline so the freeze records what's proven vs validated.

- **DECISION — freeze the LOGICAL layout, keep the PHYSICAL byte-encoding OUT of the frozen
  contract (2026-07-10, owner).** The layout freeze must draw its line between *logical* identity
  and *physical* encoding:
  - **Frozen (the observable contract):** field identity + logical order, **enum ordering =
    declaration order** (`e1 < e2` / `sorted<T[enum]>` compare by the variant's declaration index,
    NOT by its stored discriminant value), `==`/ordering semantics, the format caps (256 variants,
    65536 stores). These are the durable, portable, self-describing contract the @PLN43 mmap store
    and save files rely on.
  - **NOT frozen (a storage detail, permutable):** the concrete byte offsets and the discriminant
    *values* a variant is stored as.
  - **Why it matters — two forces need it.** (1) The durable/portable store wants a stable,
    self-describing canonical encoding. (2) **Protected paid assets** (animations/effects — almost
    all *library* work, never touching game logic) want a **per-export permuted** physical layout so
    a shipped asset file can't be ripped by a generic tool; and because they are **mmap'd**
    (zero-copy, on-disk == in-memory — no load-time decode), the permutation must BE the physical
    layout read in place. Keeping the physical encoding out of the frozen contract makes that
    per-export permutation a *legal additive transform*, not a break — the same logical types get
    two mmap-able physical realizations (canonical + permuted).
  - **The load-bearing requirement:** enum ordering MUST be defined on the **logical** index, not
    the physical discriminant — otherwise randomizing the discriminant would silently change every
    enum comparison/sort per build. This is safe *because* these assets are library-only data never
    ordered as keys, so permuting the physical discriminant is semantically transparent. The
    permutation stays **normal codegen over a permuted layout table** (build-time), never a special
    per-access codegen path — so the per-frame animation/effect loop is untouched.
  - **The mechanism (the seam):** field/enum access is a **getter axiom** — its *logical* contract
    (read field X → its value) is the frozen thing; its *physical* realization (offset, permuted
    discriminant, decode) is behind it and unfrozen, and is the one place a build specializes to its
    permutation (no per-access special codegen; `OpGetField` is already getter-shaped). The
    **decode location** (CPU-at-upload vs GPU-in-shader — the animation hot path is on the GPU) is
    likewise below the axiom and unfrozen, so a later CPU→GPU move is additive.
  - **The *why* (ecosystem + legal):** the maker ships free games/assets and needs none of this,
    but a **small indie dev selling paid assets** does — without it loft is unusable for that
    commercial slice. And the proprietary/opaque format is legally load-bearing: an opaque,
    per-export-permuted, schema-stripped, decode-compiled measure is defensibly a **technological
    protection measure**, which unlocks **anti-circumvention** protection (DMCA §1201 / EU Art. 6)
    on top of copyright + license — the self-describing canonical mode is deliberately NOT a TPM.
    Full rationale + caveats (jurisdictional; mechanism not guarantee; general info, not legal
    advice): [protected-assets.md](protected-assets.md).
  - **Action:** state enum ordering as declaration-order in `layout.md`/`matching.md`; pin field/enum
    access as a getter axiom (logical frozen, physical/decode-location unfrozen); add a
    DESIGN_DECISIONS entry that the physical byte-encoding is explicitly OUTSIDE the frozen contract
    (permutable), with the per-export protected-asset mode named as the motivating additive feature.
    Same "freeze the observable contract, leave the encoding free" shape as the null-model keystone.

### Operational / evaluation
- F2 (compound-assign double-eval), F3 (`&&`/`||` short-circuit), F4 (assignment eval order), F5
  (match guards + non-enum match) above.
- **Loop attributes `#index`/`#first`/`#count`/`#next`/`#remove` unspecified** — especially `#remove`'s
  cursor semantics (mutation mid-iteration). Pin them.
- **Spreadsheet-model observability asymmetry:** value is uniform (null+continue) but div0 *warns*
  while overflow/OOB are silent — state the asymmetry as frozen (and reconsider whether OOB should
  warn like div0; the reachability argument is weaker for OOB than overflow).
- **Format-null render is syntactic:** `c=a/b; "{c}"` → `null` but `"{a/b}"` → `null(/0)` — the same
  null renders differently by where the fault sat; hoisting drops the tag. Carry the tag on the value
  or drop it — don't freeze the syntactic split.

### Grammar / format sub-language / precedence
- **The format sub-language is the least-specified frozen sub-syntax** (highest-value): `F-Render`
  omits tuples/hash/sorted/index/ranges/closures/fn-refs/references (render impl-defined); the
  format-spec mini-grammar is incomplete (component order `{x:+08.2}`, `#b`/`#o`, uppercase hex, fill
  chars, `.P` on non-float); the spec-`:` collides with a struct literal inside `{…}`. Write the
  complete grammar + a canonical rendering for every renderable type.
- **Precedence to decide now (grouping changes are pre-freeze-only):** F8 (comparison
  non-associative); `-2 ** 2 == 4` (unary-minus tighter than power, against math/Python); `??` is
  loosest so `x ?? d == y` parses `x ?? (d == y)` (footgun on the headline null op).
- `grammar.md` omits unary/postfix precedence (`-a.b` mis-groups in the informal grammar; `~` missing);
  the "parser IS the grammar" (C82) means a parse quirk is canonical — pair the freeze with a
  **golden parse-shape corpus**, not only output/diagnostics.
- Every string literal is a format string (no raw-string form) — a raw form is additive later, lower
  urgency.

### Interfaces / capabilities / concurrency / coroutines
- **Coroutine loop-yields diverge interp (lazy) vs native (eager)** — a program that functions on
  interp (early-break drains an infinite lazy generator) doesn't on native; and the intended fix
  (CL-9 lazy native) would itself be a *regression* under the promise. Land it pre-freeze or key it
  to a future contract; don't freeze a known interp↔native divergence.
- **`par` over a hash freezes the internal bucket-walk order** — locks the hash implementation forever;
  define it as key-ordered or explicitly unspecified.
- **`par` with an impure worker is UNDEFINED + unchecked** — loft's one silent UB, no diagnostic; add
  a purity lint or register it as the single accepted UB.
- **Capability field-reads are allow-by-default** — a security posture (a host that forgets to mark a
  field private leaks it forever); reconsider deny-by-default reads, or freeze the obligation explicitly.
- **Gaps:** monomorphization termination (recursive generic → impl-defined hang); generic
  coherence/ambiguity (scope-dependent `G-Sat` with no tie-break); multi-param generics + generic
  aggregates unspecified; coroutine frame lifecycle + iterator aliasing; `par` worker-fault result +
  context-arg provenance.

---

## Spec hygiene (prose-vs-rule / calibration — fix as you go)
- E-Left prose ("both operands evaluate") false for short-circuit ops; E-Asgn prose ("RHS first")
  contradicts LHS-first; E-NullArg ("compare against the sentinel") hides the type-split equality;
  `types.md` "a slot of τ never holds a non-τ" contradicted by C85; C86 "provenance-independent" vs
  H-View; layout.md "portable" over-claims vs native-endian; O-Deps "closed/formal" vs honest-floor.
  Per the README's own rule (prose is the mistake when it disagrees), fix each so the freeze records
  the true contract.

## How the Phase 1 (language) audit runs
Same as the lib worklist: work each item as a design decision — **alternatives presented, conversion
set enumerated** — decide with the owner, land while contract 0 allows; consciously-accept → a
DESIGN_DECISIONS entry + a golden/oracle cell. Order: **(1) the null-sentinel keystone** (it changes
signatures + values, and the spec must stop denying it); **(2) the pre-freeze-only semantic + grouping
changes** (float `==`, compound-assign, short-circuit, comparison assoc, the `&v`/reference-`==`
reconciliation) — these are the true last-chances; **(3) add the missing errors** (start with the
zero-cost sentinel-collision class); **(4) the diagnostic-identity code** (unblocks improvable prose);
**(5) pin every gap** (format sub-language, aggregate/text equality, match semantics, layout format);
**(6) spec hygiene**. Land the **golden-behavior + golden-parse corpus first** so every conversion's
diff — value, diagnostic, and parse shape — is visible.

## See also
- [lib-audit.md](lib-audit.md) — the stdlib half of the pre-freeze audit (shares the keystone).
- [COMPATIBILITY.md](../../COMPATIBILITY.md) — the promise, § Before the flip, § the one-directional error surface.
- [formal/README.md](../../formal/README.md) + [formal/ROADMAP.md](../../formal/ROADMAP.md) — the spec being audited (deviation register ~closed).
- [INCONSISTENCIES.md](../../INCONSISTENCIES.md) — the language warts ledger.
