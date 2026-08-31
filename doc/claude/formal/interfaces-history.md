# formal/interfaces-history.md — the deviation register for [interfaces.md](interfaces.md)

> **The rules are next door.**  [interfaces.md](interfaces.md) states what must always be true of the
> language; this file is its TIMELINE — every place the code was measured not to do it, when,
> what it cost, and what closed it.  The two are apart because a contract a reader has to skim
> past its own history stops being a contract they can skim.  The rules doc carries the CURRENT
> state (how many are open, and which); everything below is the record behind it.

OPEN: **0** — `D-gen-1` and `D-gen-2` were both opened and closed on 2026-08-29.

⚠ **This line read `OPEN: 0` because *"a rules doc adds no code deviation"* — a claim about the
doc's GENRE, not a measurement, and the same sentence `formatting.md` carried for its whole life
until the walk that first asked found four defects there.  It has now produced one here too.**
The oracle under it (`86-interfaces.loft`, `48-generics.loft`, and the numbered scripts) is real,
but it is an oracle for the shapes those files happen to write; `D-gen-1` is what it could not
see.

### D-gen-1 — OPENED AND CLOSED (2026-08-29): the type variable was only found under two formers

`(G-Gen)` writes a generic's shape as `fn f<T>(x: …T…) -> …T…`, and the ellipsis is the rule:
`T` may sit anywhere inside a parameter type.  The DECLARATION read it that way — the check that
the first parameter carries the type variable is `arguments[0].typedef.contains_def(tv_nr)`, which
descends `Type::for_each_child` and therefore knows all seven child-bearing formers.  **The two
reads at the CALL did not.**  `Parser::extract_type_var` (*which* type variable) knew `Vector`;
`Parser::resolve_type_var` (*what it binds to*) knew `Vector`.  So a declaration the parser
accepted was one no call could reach:

| first parameter | before | after |
|---|---|---|
| `T`, `vector<T>`, `vector<vector<T>>` | ✓ | ✓ |
| `T?` | `Unknown function f` | ✓ |
| `(T, T)`, `(T, integer)` | `Unknown function f` | ✓ |
| `iterator<T>` | `Unknown function f` | ✓ |
| `vector<T>?` | `Unknown function f` | ✓ |
| `fn(T) -> …` | `Unknown function f` | ✓ (except `D-gen-2`) |

The diagnostic is the tell: *"Unknown function"* about a function declared three lines above the
call, at every instantiating type — `text`, a struct and every scalar alike, so the scalar axis
this register leans on could not see it either.

Two further homes rewrote `[T ↦ C]` over the same tree with FOUR formers each
(`Parser::substitute_type`, `Function::subst_type`), so `fn(T) -> T` in a LATER parameter was
refused with *"expected `fn(T) -> T`, got `fn(integer) -> integer`"* — the substitution the
message itself asks for.  A third copy, `Data::rewrite_type_opt`, had all seven; a fourth,
`Function::rewrite_unknown`, had five.  **One question, five homes, four different lists.**

**The corpus is why no oracle could see it, and the number is the point.** Across
`tests/scripts`, `tests/docs`, `default/` and `doc/`, **166 generic declarations put a bare `T`
or a `vector<T>` in the first parameter and not one put anything else** — exactly the two arms
the descent knew.  Implementation and tests were written against each other.  Every `T?` guard
in the tree (`1020-*`, `1023-*`) writes `fn g<T>(v: vector<T>, a: T? = null)`, putting the
carrier first; move the `T?` to the front and the same file will not compile.

Closed by deriving all four from the keystone: `Type::map_children` (the SET twin of
`for_each_child`) and `Type::zip_children` (the PAIR twin, for a walk that descends two type
trees at once) are exhaustive, so a new `Type` variant fails the build rather than quietly
staying parametric.  `extract_type_var`'s leaf also became precise — a type-var PLACEHOLDER
rather than any `Reference` — so a first parameter that names a concrete struct beside the
variable (`(P, T)`) answers with `T`.  Guard:
`tests/scripts/a-type-variable-is-found-under-every-former.loft`.

### D-gen-2 — OPENED AND CLOSED (2026-08-29, loft#1175): a fn-ref returning `T` at `text`

`fn f<T>(x: T, g: fn(T) -> T)` is correct at every instantiation measured — `integer`,
`boolean`, `character`, `float`, `vector<integer>`, a struct — and faults at `text` on
`--interpret` while `--native` answers correctly.  A call through a fn-typed slot pushes hidden
`&text` work buffers, and how many is read off the return type where the call is LOWERED, inside
the template, where the return is still `T` and the count is zero.  This is `(G-Mono)`'s
recurring class exactly: substitution rewrote the TYPE and left the COUNT behind.

Closed by DEFERRAL, the cure this register already names for its class: the count is re-asked in
`rewrite_generic_type_defaults`, where `T` is real and the fn-ref variable's type in the
monomorph's own table is concrete, and the buffers are pushed with the same builder and in the
same order as the four parse-time sites.  `args.len() == params.len()` is what says the buffers
are still missing — the visible arguments are all a site pushes when the count was zero — so a
call whose return was already concrete text is left alone rather than served twice.

Two things the deferral needs that the parse-time path gets for free, both already written down
elsewhere in the tree:

- The variables come from `caller_text_buf`, not the shared `__work_N` counter.  This mint
  happens after both passes, and drawing from the shared sequence would shift every later
  `__work_N` (loft#662's class — the reason `collections::callback_call_ref` already mints this
  way).
- A buffer minted after the parse is not declared at the top level, so `scopes::check` scopes it
  to the ARGUMENT block it appears in and frees it there, before the callee fills it.  A
  top-level `Set` is hoisted for each new one, the same replay `patch_tret_callers` performs for
  exactly this reason.  Without it the interpreter was correct and `--native` emitted a
  `String` declared inside the argument block with an empty `OpCreateStack` beside it, which
  does not compile — the divergence appearing on the OTHER backend from the original fault.

⚠ **The obvious cure was built and measured and is wrong.** `Data::fnref_text_buffers`' own doc
says its candidate test is deliberately loose because *"being loose can only mint a buffer nothing
uses, which the pop removes"* — so counting a PARAMETRIC return as a text candidate looks free.
It cured `T = text` and made all six other instantiations abort: a non-text return has no
`__retbuf` protocol for the pop to trim against, so the looseness is safe WITHIN the text family
and not across its boundary.  The guard keeps every one of those six as a cell for that reason.


- **Conformance is differential + directly checkable** — satisfaction is a single static judgment,
  so accept/reject must agree across the drivers (D-op-1's driver-agreement facet). `G-Sat`/`G-Check`
  are checkable directly (a missing method rejects on both backends); the runtime behaviour of a
  monomorphized generic is pinned by `tests/scripts/86-interfaces.loft`, `tests/scripts/48-generics.loft`,
  `tests/scripts/1028-generic-null-typed-per-monomorph.loft` and
  `tests/scripts/1032-generic-iterator-return.loft`.

- **What `OPEN: 0` rests on here — "applied throughout" is the load-bearing phrase.** `(G-Mono)`
  says `[T ↦ C]` reaches *attribute, return, and body types, and every method call*. Four
  defects have now been the same omission: an operation whose choice is a function of `τ` was
  DECIDED while `τ` was still the type variable, and substitution then rewrote the type and left
  the choice behind — loft#1016 (`x?`'s default), loft#1020 (`x == null`), loft#1028 (a `null`
  literal's conversion), loft#1032 (the yield channel a `for` over a generator is paired with).
  Each was invisible to the oracle above, because both scripts instantiate
  over records; none of the three misbehaves at `T = <a struct>`, where a reference sentinel is
  the right answer anyway. loft#1028 is the sharpest reading of that gap: it made the two backends
  disagree — the interpreter answered a `text` monomorph the empty text, `--native` refused to
  compile the program — which is the one thing this section says monomorphization cannot do.
  A scalar instantiation is therefore one axis this doc's oracle was missing, and the count
  stays 0 only as long as the tests keep one.

  **The count is now six, and the two newest were found by sweeping the OPERATION rather
  than the type** (2026-08-22).  Both `1028-*` and `1032-*` sweep `T` across the scalars,
  but each sweeps ONE operation — the null, the yield channel — so the axis left fixed
  was *which operation the template decides*.  Moving it turned up two more the same day:

  - **The `??` null CHECK.**  `== null` was deferred by loft#1020; `??` asks the same
    question and was not.  It took the placeholder's own shape (a reference) and baked
    `rec != 0`, and the after-the-fact repair listed integer / text / float / single /
    enum and ended `_ => None`, so `boolean` and `character` fell through it.  `x ?? fb`
    LOOPED FOREVER at `T = boolean` and corrupted a record at `T = character` on
    `--interpret`; `--native` refused to compile either monomorph.  All three spellings
    were affected — `x ?? d`, `x?`, and `x ?? return d` — because all three reach the
    one check.
  - **The element READ.**  `wrap_vector_get_val` picks the value-extraction op from the
    element type and ended `_ => return code`, which reads as *"everything else is
    reference-shaped"* and was not: `character` and a VALUE enum both need unpacking.
    A template's `v[1]` handed back the address as the value — a garbage codepoint for
    `['a','b']`, `null` for `[Col::Blue, Col::Green]` — on BOTH backends, while the
    concrete twin was right.

  Both are now closed, the check by deferral and the read by an EXHAUSTIVE match (adding
  a `Type` variant fails the build there rather than joining the unhandled set).  The
  guard is `tests/scripts/generic-monomorph-null-and-element.loft`, which pairs every
  boolean and character cell with its hand-written twin — `(G-Mono)` as an assertion
  rather than as a claim.

  **A seventh, from asking the same question of the WRITE side** (2026-08-22).  The
  element read was one operation; the element WRITE is another, and its corpus holds a
  different axis fixed — not the type, and not the operation, but the *spelling*.  P241's
  rewriter re-emits a monomorph's vector writes, and every test of it since 2026-05 uses
  `o += [x]`; nothing used `v[i] = x`.  An append emits a three-op sequence the rewriter
  matches, an indexed assignment emits a LONE `OpCopyRecord`, and that one reached the
  monomorph carrying the type variable's record id: at every scalar type the run PANICKED
  in the allocator, and for a struct parameter it silently wrote nowhere and read the old
  element back.  Closed by routing both spellings through the one setter builder, guarded
  by `tests/scripts/generic-vector-element-write.loft`, which sweeps spelling × type ×
  vector origin.

  The three together say the axis to sweep is not fixed: it was the TYPE for #1028, the
  OPERATION for the `??` check and the element read, and the SPELLING for the write.  What
  they share is the question — *what does this corpus never vary?* — and that question is
  the instrument, not any particular answer to it.  The lesson generalises past this doc: **`_ => None` and
  `_ => return` are how a decision that is a function of `τ` goes missing quietly**, and
  a missing arm looks exactly like a deliberate one until something reads the answer.

  loft#1032 is the same reading a second time, and adds a **third** thing the oracle did not
  carry: a RETURN TYPE that is not the bare `T`. `substitute_type` had arms for `vector<T>`,
  `(T, T)` and `T?` and none for `iterator<T>`, in BOTH twins — the parser's and the variable
  table's — so a generic returning a generator kept the type variable in its return and in the
  handle its caller bound, while the loop variable beside it was substituted. `(G-Mono)` names
  the return explicitly, so this was a deviation and not a boundary; the rule did not move. The
  scalar axis is again what made it visible: at `T = text` or a struct the DbRef yield channel
  is the right answer anyway, so every cell of the new script passes before the fix at those
  types. Two of the three other defects the same repro surfaced were NOT monomorphization
  deviations at all — a forward call's back-patch and `--native`'s argument-hoist path each
  broke for a generator with no generic in the program — which is the loft#1029 lesson
  restated: a generic corpus is where such a thing becomes visible, not where it lives.

  The corpus is thin on a **second** axis, and loft#1029 is how that surfaced: it varies the
  instantiating TYPE and never varies how the ARGUMENT is spelled. Every call in both scripts
  binds its argument to a variable first, and a fresh-arm/borrow-arm join reached with anything
  else — a literal, a field, an element, a `??` — leaked a record on both backends until
  2026-08-20. That defect was NOT a monomorphization deviation — it reproduces with no generic in
  the program at all, so it is `ownership.md`'s to own (D-own-6, now closed) — but it was a
  generic corpus that made it visible, and the same omission would hide a monomorph-only variant
  of it here. `(G-Mono)`'s promise is that a specialised copy behaves as the hand-written
  concrete one would; an oracle that fixes the argument spelling cannot see the cases where it
  would not. The generic spelling itself is now a probe under
  `tests/scripts/1029-inline-argument-borrow-source.loft`'s finding and measured clean.
- **Test-hygiene note (resolved 2026-08-09):** `86-interfaces.loft::test_bounded_for_loop_struct`
  — a bounded `<T: Validatable>` for-loop over a struct vector calling a method per element — was
  commented out under a stale "crashes with P136 (use-after-free)" note. That bug is FIXED and the
  guard is live: `loft --tests tests/scripts/86-interfaces.loft` runs 11 functions including it,
  green. Only the trailing "Uncomment when fixed" comment beside it is left over.
