---
render_with_liquid: false
---
# Destination-passing — the chokepoint that retires `scratch`

**Design Protocol 1 artifact (the written prediction).**  Session-2 on branch
`strings`, 2026-06-05.  The README's session-1 handoff named the wrong next
slice; a boundary matrix on `--interpret` overturned its premise.  This doc is
the corrected design, with every load-bearing claim probed.  It is a
*hypothesis to validate by building*, not a committed plan.

---

## 0. The matrix corrected the scope (verified, opcode-level, `--interpret`)

One text-producing shape per isolated probe; dispatch read from the execution
trace (`StaticCall(..._dest)` + `InitCreateStack` = destination-passing;
`StaticCall(t_4text_...)` = scratch).

| Shape | Dispatch today | Scratch? |
|---|---|---|
| `s = x.to_lowercase()` (let) | `…_dest` | **no — already handled** |
| `s = x.to_lowercase()` (reassign) | `…_dest` | no |
| `out += x.to_lowercase()` | non-dest | **yes** |
| `b.name = x.to_lowercase()` (field) | non-dest | **yes** |
| `println(x.to_lowercase())` (arg) | non-dest | **yes** |
| `fn f() -> text { x.to_lowercase() }` (return) | non-dest | **yes** |
| `a.to_lowercase() + b.to_uppercase()` (concat) | non-dest | **yes** |
| `"x={a.to_lowercase()}y"` (format) | non-dest | **yes** |
| `a.to_lowercase() == "ab"` (compare) | non-dest | **yes** |
| `v += [a.to_lowercase()]` (vector push) | non-dest | **yes** |
| `a.to_lowercase().to_uppercase()` (chain) | non-dest | **yes** |
| `if c { a.lower() } else { a.upper() }` (cond) | non-dest | **yes** |

**The plan's "next slice" (`s = native()`) is already shipped.  The plan's
"already-handled" case (`out += native()`) actually leaks.**  Destination-passing
today is *one narrow special-case* bolted onto the `set_var` text path
(`codegen.rs:3200` → `gen_text_dest_call`); a second, parallel attempt
(`try_text_dest_pass`, `codegen.rs:2247`) exists for `OpAppendText` but silently
does not fire (see §3, Claim C).

## 1. The three load-bearing claims, probed

**Claim A — the hazard model — CONFIRMED three ways.**  `OpClearScratch` is
**never emitted**: the loft `fn OpClearScratch()` (`default/02_files.loft:441`)
is an empty no-op; the Rust `clear_scratch` (`fill.rs:279`) sits in `OPERATORS`
but is bound to **no** library name (`native.rs FUNCTIONS` doesn't list it) so it
is unreachable; and the codegen guard `library_names.get("OpClearScratch")`
(`codegen.rs:329`) returns `None`, so the per-`Line` emission is dead.  ⇒
**scratch is an append-only, never-reclaimed `Vec<String>`.**
- The plan's *leading* hazard (§2.2 cross-statement dangling) is closed **by
  accident** — nothing is freed, so nothing dangles (this is why
  `tests/scripts/192` is green).
- The *live* hazard is the §2.1/2.3 **leak**, and it is **worse** than the plan
  states: unbounded over the whole process lifetime, not bounded by one
  statement.
- **Therefore you must NOT "re-enable the clear."**  Re-enabling reintroduces the
  dangling hazard that is currently closed-by-accident.  The only correct fix is
  the plan's strategic direction — **eliminate the producers**.

**Claim B — "a destination chokepoint already exists" — FALSIFIED.**  Only the
single direct `s = native()` assignment dest-passes; all 11 other shapes scratch
(§0).  The current code is nowhere near unified.

**Claim C — why `try_text_dest_pass` doesn't fire for `+=` — RESOLVED, and it is
the design's smoking gun.**  `out += x.to_lowercase()` lowers to
`OpAppendText(Var(out), Span(Call(to_lowercase,[x])))` (`operators.rs:120`).
`try_text_dest_pass` matches `parameters[1]` as `Value::Call` **without
`unspan()`** (`codegen.rs:2251`); the `Span` wrapper makes the match miss → it
returns false → scratch.  **This is the same `Span`-unspan footgun that already
bit P217** (documented at `operators.rs:83`).  A per-shape special-case, silently
broken by a wrapper.

## 2. The invariant

> *At codegen, a text-returning native always writes its result into a
> destination buffer the call site provides; the runtime never holds a produced
> `text` the source doesn't imply (no global `scratch`).*

## 3. The alarm (Protocol step 2): N ≥ 7 silent re-assertion sites

Destination-passing is re-implemented **per consumer-shape** — `set_var`,
`try_text_dest_pass`, field-write, arg, return, the `emit.rs` wraps, the cdylib
bridge.  Each independently re-states "match a text-native call, find a
destination, route to `_dest`," and each is **independently vulnerable**: the
`Span` footgun hit `try_text_dest_pass` but not `set_var`.  **Omission is
silent** — a missed site falls back to scratch, which "works" only because
scratch is never cleared (Claim A).  The plan's Phase-A is the spray:
`N_natives × N_shapes`.  `N × silence` is the brittleness, known now.  Cure:
**collapse N toward 1** (one chokepoint) + **make omission loud**.

## 4. The family structure (Protocol step 4 — over-unification guard)

Probing the cleanest/most-dangerous claim — *"one producer-side mechanism serves
every consumer."*

- **Families 1 (named destination reachable) + 2 (no destination, consumed
  within the current scope) COLLAPSE** to one decision at the text-native-call
  lowering: *does the consumer trivially provide a text slot?  yes → write into
  it; no → synthesize a scope-bound stack-temp.*  Covers assignment, append,
  field, vpush, concat, cond (family 1) and arg, compare, format-piece,
  chain-intermediate (family 2).  **[prediction — the build settles whether each
  destination-less position truly shares one synth-temp mechanism]**
- **Family 3 (escapes the current scope) is GENUINELY separate** — falsified the
  unification here: `fn f() -> text { native() }`, closure capture, coroutine
  yield each need the text to **outlive the scope**, so the destination is a
  caller-owned work buffer / store record, not a local temp.  Already on
  `text_return`/`ref_return` (return) and Phase C (closure/yield).

So the design is **2 chokepoints + M mechanical `_dest` variants**, not the
plan's `N_natives × N_shapes` spray and not a single false-unified primitive.

## 5. The design

**One routing primitive at the text-native-call lowering.**  When the callee
returns `text`: ensure a destination — the consumer's named slot if it trivially
has one, else a synthesized scope-bound stack-temp text record freed by the
existing `OpFreeText` scope machinery — and route the call to its `_dest`
variant.  This **subsumes** `set_var`, `try_text_dest_pass`, field, arg, compare,
format, vpush, chain, cond.  Make omission loud: a text native reaching codegen
*without* a `_dest` variant (or without a destination) is a build-time assert,
not a silent scratch fallback.  Per-native work is only the mechanical `_dest`
variants (genuinely M; each ~5 lines mirroring `t_4text_to_lowercase_dest`).
Then delete `scratch`, the dead `clear_scratch`, the no-op `OpClearScratch`, and
the dead `codegen.rs:329` emission.

## 6. The build prediction (to validate — Protocol steps 5–6)

**Decisive first build:** implement the synth-temp branch for ONE
destination-less position — `println(native())` (arg-position).  **Prediction:**
the *same* primitive makes `compare`, `format`, `chain`, `cond`, `concat`
dest-pass with **zero per-shape code**, on both backends, with
`tests/scripts/192` green and the corresponding `scratch.push` producers gone.

**The falsification:** if any of those shapes still needs its own handling, the
families do **not** collapse — that is over-unification surfacing, and the
extra shape is a real domain axis to record (not to force).  The build is the
only thing that can tell the difference; desk reasoning cannot.

Acceptance bar unchanged from the README: `scratch.push` 39 → 0, then delete the
field + assert it stays empty.  But the **order** is corrected: append/field/arg
+ the destination-less family first (they leak today); the already-done
assignment shape is not the slice.
