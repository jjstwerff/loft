# formal/binding-history.md — the deviation register for [binding.md](binding.md)

> **The rules are next door.**  [binding.md](binding.md) states what must always be true of the
> language; this file is its TIMELINE — every place the code was measured not to do it, when,
> what it cost, and what closed it.  The two are apart because a contract a reader has to skim
> past its own history stops being a contract they can skim.  The rules doc carries the CURRENT
> state (how many are open, and which); everything below is the record behind it.

OPEN: **0** — D-bind-11 and D-bind-16 CLOSED 2026-09-03 (below); D-bind-12, D-bind-13,
D-bind-14 and D-bind-15 each opened and CLOSED the same day.
D-const-2 opened and CLOSED the same day (2026-09-01), found by the Store Locks
reference review.
B-Ref-Reshape is enforced for all three of B-Disturb's events (D-bind-9,
opened and closed 2026-08-05); B-Ref-AnnotationOnly is enforced in every position, not
only the ones a leading `&` reaches (D-bind-10, 2026-08-09).

> **D-bind-16 — CLOSED 2026-09-03 (opened the same day, loft#1321) — `(B-Copy)` did not hold
> when the right-hand side is a branch JOIN.**
>
> `b = if c { a } else { [0, 0] }` ALIASES `a`, and so does the struct spelling, while the
> plain bind of the identical value one line away copies. Present on the shipped 2026.8.0,
> so long-standing rather than a regression, and identical on both backends.
>
> **It was filed as part of D-bind-15 and is not.** loft#1319's matrix carried a `??` column
> described as *"the same defect discharged"*; the control above has no nullability in it at
> all and aliases the same way, because `a ?? d` lowers to `if !isnull(a) { a } else { d }`.
> So the axis is the JOIN and the `?` was a passenger — and D-bind-15's fix leaves these
> cells exactly where they were, which is the other half of that measurement. Two defects
> sharing a symptom read as one until a cell moves only one of them.
>
> The destination ends up DEPENDING on the source (`b: vec<int> deps=[a]`) rather than
> owning: `classify_vec_bind` recognises a bare `Var` and an owned field read and nothing
> else, and the record side keys on `Value::Var` in both backends. That dep is what closes
> every copy path downstream — `owned_ref` requires `depend().is_empty()`, so the
> `OpBindOrCopy` arm written for exactly this shape is unreachable. The ownership ORACLE
> reports the joined binding `Owned` throughout, so the analysis has the right answer and the
> shipped fact does not.
>
> **An attempt was made, measured, and REVERTED (2026-09-03), and what it established is the
> useful part.** The rule it implemented — a join read ARM BY ARM, copying where every arm
> would have copied on its own — is right, and three facts the arm walk needs were not
> obvious:
>
> * reading the JOIN rather than its arms is wrong. `v[i]` is `τ?`, so the ordinary element
>   read is `c = vv[0] ?? [0]` — a branch. Copying it makes `(B-View-Depth)` unreachable for
>   its own documented spelling, and `bind-copies-or-views-the-whole-boundary.loft` goes red
>   on the cell that exists to say so.
> * a `??` HOISTS its subject into a temp (`__ncc_N = vv[0]`), so the arm the walk reaches is
>   a bare `Var` and the projection is one statement up. The walk has to resolve a temp
>   against what its block bound.
> * `use_analysis::is_projection_op` does not name `OpGetVectorNullable`, while
>   `generation::hoist::ELEMENT_ADDRESS_OPS` pairs it with `OpGetVector` for the same
>   question. Two one-homes, one notion, and reading only the first answers "not a place" for
>   every nullable element read.
>
> The third is why it was reverted: **a CALL arm may return a BORROW.** `it = get(b) ?? d`,
> where `get` answers a view of its parameter, has no syntactic projection in any arm, so the
> walk said "copy" and the caller then freed at scope exit what it had borrowed — loft#974's
> guarded behaviour, caught by `accessor_borrow::an_accessors_returned_view_names_its_parameter`.
> Trading a silent alias for a wrong free is not an improvement.
>
> **CLOSED 2026-09-03, with the copy face and the free face as one change.**  The rule the
> attempt implemented stands; what changed is WHERE it is applied.  Instead of deciding the
> join's copy and then re-deriving each arm's copy in both backends, each qualifying arm tail
> is rewritten into the bound spelling on a temp — `{ __lift_N = a; __lift_N }` — and the temp
> is bound by the SINGLE bind's own lowering (`scopes.rs::lift_join_arm_tails`, ownership.md's
> D-own-8 record has the whole arm-kind table).  The three facts above are then honoured for
> free: a `??` hoist is a variable the join hands back and is left alone unless its subject
> was a call the caller must copy; the join is never read whole; and a call arm that returns a
> borrow is bound by the same codegen arm that copies `it = get(b)` — no syntactic walk decides
> anything.  A record arm copies at the bind on its first and every later execution; a vector
> arm is refilled into a function-scoped buffer by `OpReplaceVector`, whose element type the
> scope pass now reads from the store registry it was handed for the purpose.
>
> Measured on both backends: the filed matrix (vector and struct, `if` and `match`, the `??`
> spelling), a parameter arm and a loop-element arm against their plain binds, the accessor and
> closure view arms (which also closes an interpreter/native split: the interpreter viewed and
> native copied), a null nullable subject binding its default, and the return position.
> `(B-View-Depth)`'s `c = vv[0] ?? [0]` stays a view.  Guard
> `1321-a-joined-binding-copies-what-a-plain-bind-copies.loft`, falsified at 26d17f4b.
>
> So the predicate the next attempt wants is not the syntactic walk but *"does this arm's
> VALUE borrow?"*, which the return TYPE already answers — loft#974 is the change that put
> the dep there (`-> Y974?["b"]`).

> **D-bind-15 — CLOSED (2026-09-03, loft#1319) — `(B-Copy)` did not hold for a NULLABLE
> heap local: a whole-value bind ALIASED its source.**
>
> `b = a` with `a: vector<integer>?` aliased `a`, and so did a nullable struct, while the
> keyed kinds copied — which is what said the axis was the `?` and not "heap". None of the
> rule's three exceptions reaches it: this is a whole value off an OWNED local, not a struct
> projection (B-View), not a borrowed base (B-View-Base), not an index or nested read
> (B-View-Depth).
>
> **One cause, and it is the spelling again.** `τ?` is `Optional(τ)` — the same storage
> behind a nullability marker (`@FR-L-Null`) — and FOUR sites decided the lowering by
> matching the `Type` variant BARE, so the wrapped shape reached none of them and the
> default (alias) stood: the vector-bind selector and its consumer, the interpreter's
> first-set dispatch, and the native generator's whole-record bind. D-bind-13 is the same
> sentence one construct over, and the keyed kinds took this peel in loft#1143 — so this is
> the third time the register records it, and the sites were siblings of ones already fixed.
>
> **The second half is that a copy must not turn ABSENCE into EMPTINESS.** A null source has
> to leave the destination null, not holding the store the copy allocated for it. Both
> mechanisms already existed and are reused rather than restated: `Stores::vector_replace`
> gains the guard `replace_keyed` has carried since loft#1150, and the record bind routes
> through `OpBindOrCopy`, whose borrow arm materialises and whose other arm adopts — which
> for the null sentinel is exactly "stay null". `OpCopyRefOrNull` was tried first and is
> wrong here: it binds `Stores::null()`, whose `store_nr` is a REAL slot with `rec == 0`,
> while `x == null` on a record lowers to `OpRefIsNull` and tests `store_nr == u16::MAX`.
> The two spellings of absence agree for the element read it was written for and not for a
> bound local.
>
> **Why eleven cells and both backends read green over it.**
> `tests/scripts/bind-copies-or-views-the-whole-boundary.loft` is the one place the
> copy-vs-view boundary is pinned, and every one of its eleven subjects was declared
> non-null — `??` appeared in it only inside element-read assertions. The axis it never
> moved is the one that broke. It has the nullable-subject axis now, and the four added
> cells fail on the pre-fix build.
>
> Guard: `tests/scripts/1319-a-nullable-whole-value-bind-copies-like-its-dense-twin.loft`,
> whose controls are `&` (must still alias), the struct projection and index read (must still
> view), the collection projection off an owned base (must still copy) and every keyed kind
> (must not move).

> **D-bind-14 — CLOSED (2026-09-03, loft#1316) — `(B-Ref-StoredRef)` admitted its one
> position only when the field came LAST, and not at all once the field was nullable.**
>
> The rule names exactly one place a prefix `&` is legal outside a `&τ` binding — a
> struct-literal field whose declared type is `reference<τ>` — and it conditions that on the
> FIELD'S TYPE and on nothing else. Two things were read instead.
>
> **The terminator.** The gate accepted the `&` only when the next token was `;` or `}`, which
> is what ends an ASSIGNMENT right-hand side. A field value also ends at the `,` before the
> next field, so `Trail { l: &pool[0], n: 4 }` was refused while the identical literal with
> the fields swapped compiled. Field ORDER is not in the rule, and a reader hitting this reads
> it as "`&` does not work here" rather than "`&` does not work last-but-one".
>
> **The type.** The same gate matched `Type::Reference` unpeeled, so a field declared
> `reference<τ>?` — whose bytes `L-Null` makes identical to `reference<τ>`'s — was not a
> `reference<τ>` field as far as the gate was concerned, and the `&` the pointer field exists
> for had no spelling once the `?` was written. That is D-layout-4's mechanism reaching this
> doc: one IR spelling, two source notions, and a site that reads neither the share marker nor
> `base()`.
>
> **Status — CLOSED.** The position is named rather than inferred: `AmpHead` is `No`,
> `AssignRhs` or `StoredRefField`, and the terminator set is read off it, so "which tokens end
> this operand" is answered once per position instead of once per gate. The type test peels.
> Guard: `tests/scripts/1316-a-nullable-reference-field-is-still-a-pointer.loft` (the nullable
> half, both backends) and `tests/scripts/150-amp-head-position.loft` (the position family,
> which already owned the `&`-in-a-field cell and gains the not-last one).

> **D-const-2 — CLOSED (2026-09-01) — `(Const-Value)` went unenforced on two
> append routes, and both mutated the CALLER while the parameter said `const`.**
>
> `fn f(p: & const vector<integer>) { p += [9]; }` compiled, and the caller's vector grew.
> So did `fn add(p: const hash<R[k]>) { p += R { … } }`, and its `sorted` and `index`
> twins. Both backends, exit 0, no diagnostic. `(Const-Value)` says the value behind a
> value-const name is read-only and **every** through-write is rejected, so both are
> deviations and neither was a design question.
>
> **One cause: the guard was attached to the lowering ROUTE rather than to the write.**
> `parse_assign_op_inner` picks among a dozen routes by target shape, and each route that
> could reach a const binding carried its own copy of the check — the vector builder, the
> keyed builder, the two text paths, and a `Value::Insert` bypass added when the struct
> constructor was found to miss the others. A per-route guard is exactly as complete as
> that route's target-shape test, so every shape a route declines falls through unchecked,
> silently. The vector route destructured `f_type.base()` against `Type::Vector`, and
> `base()` peels `Optional` but not `RefVar` — so the `&` spelling of a vector parameter
> was never asked. The keyed routes had no check at all.
>
> **The cure is that the question is not the route's to ask.** Whether a write is allowed
> is a property of the BINDING; the route only decides how it is lowered. One
> `guard_const_write(var_nr, op)` ahead of the dispatch replaces all five copies, which is
> also why the fix DELETES code. Every diagnostic keeps its wording; the one observable
> change is that a `;`-terminated statement now reports the same COLUMN as the same
> statement without one, because the guard no longer runs after the terminator has been
> consumed (`a_diagnostic_names_its_own_line_*` pins all three layouts).
>
> **Why it stayed unfound.** The rules' own oracle — `40-const-fields.loft` plus the
> `pln40_*` negatives — crosses `const` with the four quadrants and with struct-vs-enum,
> and with nothing else: no `&` cell, no keyed collection. An OPEN count is only as strong
> as the crossings under it. `tests/scripts/const-binds-through-every-append-route.loft`
> is that crossing, measured cell by cell against the pre-fix build.

> **D-bind-13 — CLOSED (2026-08-26, loft#1106) — `(B-Copy)` did not reach a bind whose
> destination was NULLABLE, and the same blindness left the callee's minted store
> unowned.**
>
> `r = pick(q, c)` where `pick(a: P?, …) -> P?` ALIASED its argument: `r?.x = 55` set `q`.
> The identical call written `-> P` copies, which is the oracle. `(B-Copy)` is not
> ambiguous here — a call RESULT is a whole value, not a projection, so none of B-View /
> B-View-Base / B-View-Depth reach it — and the same shape leaked one record per call on
> the arm where the callee minted its own store.
>
> **One cause, and it is a spelling.** `P?` is `Optional(Reference(P))`: the same storage
> behind a nullability marker. Every shape question in the heap first-bind dispatch —
> `gen_set_first_at_tos`'s arm list, `generation::dispatch`'s `heap_def_nr`, and
> `scan_set`'s `record_target` — was asked against the BARE type, so the nullable spelling
> reached none of them and the bind stayed a plain adopt of the returned `DbRef`.
>
> **The trap this issue carries, and it is worth stating.** The clean-looking twin
> (`q: P? = …`) was clean BY ACCIDENT: its dep was dropped only because the caller-side
> dep resolver has no `Optional` arm either, so the local read as an owner and earned a
> free. Classifying `Optional` "properly" without the runtime guard turns that clean case
> INTO the leaking one. So the deps strip and both backends' guard read ONE predicate,
> `use_analysis::nullable_join_first_bind`: a strip always has a guard under it, and a
> guard always has the free it was emitted for.
>
> **The fifth spelling was the one that hid.** `Function::make_independent` — the REMOVE
> half of the dep list — had its own inline arm list, and it too had no `Optional`. So a
> nullable local's dep could be READ (`Type::depend` peels) and SET (`Type::with_deps`
> peels) but never CLEARED: no `S?` could be made an owner by any caller, and the strip
> above was a silent no-op until this was folded onto `Type::deps_mut`. A dep list reached
> through three faces has to peel on all three.
>
> Enforced at: `use_analysis::nullable_join_first_bind` (the question), `scopes::scan_set`
> (the strip), `state/codegen::gen_set_first_at_tos` and `generation/dispatch` (the guard),
> `Type::deps_mut` (the one home for the mutable dep list).
> Guard: `tests/scripts/1106-a-nullable-heap-local-owns-its-bind.loft` — 15 cells including
> the null-answer arm, the struct-enum spelling, the nullable COLLECTION return (a
> different delivery, untouched) and an argument that OUTLIVES the call.

> **D-bind-12 — CLOSED (2026-08-23) — the struct write-back was a real defect; the
> collection alias was the RULE being under-stated.** Filed as two halves; measuring the
> second one properly split them apart.
>
> **Half one — FIXED.** Writing back a value BOUND from a sibling element vanished from the
> IR entirely and leaked the record with it: `for p in w { hs = p.0; p.1 = hs; }` left
> `w[0].1` unchanged, on both backends. `move_elidable_source`'s last gate is *"owns a
> transferable store"*, read off `Uses::def_vdb` — whose own doc says *`v = OpGetField(vdb,
> 0, _)` where vdb is OpDatabase'd* and whose walk never checked the second half. So `hs =
> p.0`, a read of an EXISTING element through a borrow, counted as owning one, and
> `move_rewrite` dropped the `OpCopyRecord`. That drop is sound only when the source is
> CONSTRUCTED — its build ops are retargeted onto the destination — and `hs` has no build
> ops, so the copy WAS the write. `collect_uses` now enforces the documented condition once
> the whole body has been walked (the `OpDatabase` may not have been visited at insertion
> time). Precisely scoped: emitted IR is **byte-identical on 120 of 120 scripts**, and
> `857`'s own allocation count is unchanged at 27, so the pointer-bind it protects is
> untouched.
>
> **Half two — RESOLVED (2026-08-24): `B-View` was under-stated, and the missing clauses are
> now written as `B-View-Base` and `B-View-Depth`.** It is NOT the owner question this entry
> first called it: `OWNERSHIP_MODEL § The law` and #426's RESOLUTION had already decided it,
> and #426 records that its own filed premise (*"an index / nested read must COPY"*) was the
> wrong read. So the code was right and the rules doc was incomplete. The whole boundary — 11
> cells, both backends — is pinned by
> `tests/scripts/bind-copies-or-views-the-whole-boundary.loft`.
>
> **Original reading, kept because the mistake is the useful part:** `hv = p.0` on a
> COLLECTION element aliases, and the first reading scored that against `B-Copy`. Measured
> across the 2×2 off a BORROWED base, three of four projection cells are views:
>
> | construct | element type | behaviour |
> |---|---|---|
> | struct field | vector-typed | view |
> | struct field | struct-typed | view |
> | tuple element | vector-typed | **view** — the cell that was filed |
> | tuple element | struct-typed | copy |
>
> The implemented model is *a projection off a borrowed base is a VIEW; off an OWNED base it
> COPIES* — gated explicitly by `classify_vec_bind`'s `depend().is_empty()`, deliberate
> (`cells = sc.v; cells[i] = h` writing through is @PLN25 p379's point), and with its
> alternative measured to CORRUPT (#426, `185-nested-boolean-vector`). Verified in both
> directions: an owned base copies (`a = h.items` ⇒ `[1,2]`), a borrowed one views
> (`b = s.vecf` ⇒ `[9,9]`), and the p379 write-through reaches the source.
>
> `B-View` above states the view for a **struct-typed** projection only, so the rules cannot
> express a model the language depends on. Per [README](README.md) that means the RULE wants
> extending, not the code changing — **a rules question for the owner, deliberately not
> decided here**, since widening `B-Copy` instead would delete p379's idiom and re-enter
> #426.
>
> ⚠ **The fourth cell was the one deviation, and `B-View` already settled its direction —
> FIXED the same day.** A STRUCT-typed tuple element copied while its three siblings viewed,
> and `B-View` says a struct-typed projection IS a view, so there was no decision to make:
> the code had to move. The stored-tuple element read took the synthetic struct's attribute
> type VERBATIM, carrying neither the base's deps nor the base variable — so the bind typed
> as an OWNER while holding a handle into someone else's record, and was handed an
> `OpFreeRef` to match. Its two siblings already did it right and one says why: the
> plain-tuple site's P197 comment (*"without this, `a.v.0` returns a `Str` whose ptr points
> into a freed host"*), and `fields.rs`'s struct-field read, which carries the base deps AND
> `depending(base_var)`. All four projection cells are now views, the bind carries `["p"]`,
> and the spurious free is gone. Precisely scoped: emitted IR is unchanged on **80 of 80**
> tuple-bearing scripts (the only file that differs is the guard's own).
>
> **The consequence is pinned rather than left to be discovered:** a three-step swap through
> a bound element does NOT swap (`held` names the place), which is what its three siblings
> already did. `test_swap_through_a_view_does_not_swap` asserts that, and
> `test_swap_by_holding_the_value` shows the cure — hold the VALUE (a scalar/text local) and
> rebuild after the write.
>
> Guards: `tests/scripts/reference-tuple-heap-element-through-a-record.loft` — 8 cells, the
> two write-back ones proven to fail on a pristine worktree at `c3d18a5f` while the shapes
> that always worked (`p.1 = p.0`, a fresh literal) pass there, which is what made this read
> as *"writes to `p.1` are fine"*.

> **D-bind-11 — CLOSED (2026-09-03, loft#1006): a `&(τ, …)` holds any element a struct field
> can, and the two options the entry below named were never a choice.**  The stack form cannot
> OWN a `text` element — a 16-byte `Str` borrow has no owner of its own in a frame — while the
> record form already performed this exact swap correctly on both backends through the loop
> variable over a `vector<(text, text)>`.  So a `&(…)` whose elements are not all scalars is a
> reference to the synthesized `__tuple<…>` RECORD — precisely what a `&S` is — and the one
> refusal site, `Parser::ref_var_type`, builds that type instead of refusing.
>
> **The whole distance was the LITERAL local.**  A tuple local bound from a heap-tuple RETURN
> was already that record; a loop variable was already that record; only `a = ("a1", "b1")` was
> stack-built.  Making EVERY heap-tuple literal local a record was measured and rejected: 17
> corpus scripts changed exit code, two of them crashes — nullable elements, fn-ref elements,
> nested and forward-referenced tuples, destructure ownership, none of which a `&` ever touches.
> The representation moves only for a tuple local that is the SOURCE OF A LINK: recorded in
> pass 1 at the link (`b = &a`, `c: &(…) = a`) and at the call argument, applied at the bind in
> pass 2 (`Parser::ref_linked_tuple_locals`, the `adopted_ret_defs` pattern), built by the
> return path's own `rewrite_tail_tuple_with_work_ref`.  With that scope the corpus answers
> identically with and without the change — measured script by script against the pre-change
> binary.
>
> ⚠ **Three cuts the matrix caught.**  Pass 1 typed the linked local as the stack tuple, and
> the record RHS was then UNBOXED back to it (`unboxes_stored_tuple`), so the link saw a stack
> tuple on pass 2; the unbox is declined for a linked local.  An annotated `c: &(text, text) =
> a` failed on pass 1, where `a` was still a stack tuple and the link's derived type disagreed
> with its own annotation; the link derives the record type as soon as the fact is recorded.
> And `slow-reference-parameter` fired for `&(text, text)` — advice whose premise (a by-value
> parameter already propagates writes) is FALSE for a tuple, whose by-value form is a copy
> (`1278-…`); suppressed for a `__tuple<…>` reference.
>
> **What stays refused, and each is a cell in `102-…`:** a NULLABLE element (its `?` does not
> survive the synthetic name), a fn-ref element, a nested tuple — the record cannot spell or
> lay them out as a field; a by-value tuple PARAMETER or a FIELD as the source (no local to
> build the record for, `T-Ref-Src`); and a `&(…)` containing a type declared LATER in the
> file, refused the way a tuple return is (`refuse_forward_ref_tuple_params`, which has to
> match the RESOLVED type — `resolve_adopted_stubs` has already replaced the stub by then).
>
> The rule moved with it: [tuples.md](tuples.md) gains `(T-Ref-Rep)` (which representation a
> `&(…)` names) and `(T-Ref-El)` now admits what a struct field can.  Measured on both
> backends, `LOFT_POISON` clean, no native leak: a literal local, a return-bound local, a loop
> variable, a local link in both spellings, `text` / vector / struct / keyed / three mixed
> elements, a linked local re-bound, copied, destructured, appended and passed to two callees
> 500 times.  Guard: `tests/scripts/reference-tuple-heap-elements-link.loft`, seven cells,
> falsified at 29743c5c.
>
> **D-bind-11 — OPEN (2026-08-19) — `&(τ, …)` admits only SCALAR elements, against
> B-Ref-Alias and B-Ref-Uniform.** `B-Ref-Alias` says the `&τ` annotation makes **ANY**
> binding — scalar OR heap — a live link, and `B-Ref-Uniform` says a `&τ` variable is used
> exactly like a `τ` one. A reference TUPLE obeys neither once an element is not a scalar:
>
> ```loft
> fn sw(p: &(text, text)) { t = p.0; p.0 = p.1; p.1 = t; }   // refused at the signature
> fn sw(p: &(integer, integer)) { … }                        // fine, both backends
> ```
>
> Since 2026-08-24 the admitted set also contains a VALUE ENUM, which has a `boolean`'s exact
> 1-byte layout and was excluded only because two spellings of "is this a scalar" had drifted
> (`data::is_scalar` is now the one home). The refusal for a heap element stands and names the
> element type.
>
> ⚠ **Re-measured 2026-08-23, and the SECOND of the two named options is already running.**
> This entry says closing it needs *"either an op family that writes the STACK form through a
> DbRef, or backing a `&(…)` carrying heap elements with a real record"* — and the
> record-backed path is not hypothetical. A `for p in v` over `vector<(text, text)>` performs
> the EXACT swap `fn sw(p: &(text, text))` is refused for, correctly, on BOTH backends
> (`[("a1","b1")] → b1|a1`). So the open question is not *"can a reference tuple carry heap
> elements"* — it demonstrably can — but the narrower *"can a `&(…)` PARAMETER or LOCAL be
> given the record backing the loop path already uses"*, which D-tup-2 made pointed by
> deliberately making tuple locals STACK-backed so `&` works for scalars.
>
> The SIGSEGV below still reproduces (re-measured the same day with the `text` arms re-added),
> and its cause is now stated one level down: a `text` element on the STACK is a 16-byte `Str`
> — `{ ptr, len }`, a raw BORROW — while the record form is a 4-byte handle, so the record ops
> read a `Str` as a handle and get a corrupt record number. That is also why `fn f(s: &text)`
> WORKS while `&(text, text)` cannot: the `&text` parameter writes into the caller's 24-byte
> owned `String` via `OpClearStackText`/`OpAppendStackText` and the owner never changes,
> whereas a tuple's text element has no owner of its own on the stack.
>
> ⚠ **That working record-backed path had NO guard** — no script in the corpus wrote a `text`
> tuple element through it — so the evidence this entry now rests on was one refactor away
> from vanishing silently.
>
> **The remaining half is a REPRESENTATION choice, and the ops are what force it.** With the
> offset corrected, adding the `text` arms still SIGSEGVs — measured — because the two element
> paths speak different op families:
>
> | | addresses | representation of a `text` element |
> |---|---|---|
> | plain tuple (`OpPut*` + frame position) | a slot in the CURRENT frame | 16-byte inline stack form |
> | reference tuple (`OpSet*`/`OpGet*` + DbRef + offset) | any frame, via the link | 4-byte record handle |
>
> A callee must write the CALLER's frame, so only the DbRef family can reach it — and that
> family speaks the record form. Scalars are immune because an `i64` is 8 bytes in both. Closing
> this needs either an op family that writes the STACK form through a DbRef, or backing a
> `&(…)` carrying heap elements with a real record. That is the decision `D-tup-1` records as
> missing, and it is why the refusal stands meanwhile.
>
> `tuples.md` states no rule for `&(…)` at all, which is how a composition of two specified
> features went unspecified; see its Deviations note.
>
> ⚠ **Narrowed 2026-08-23 — a SECOND B-Ref-Alias violation was sitting behind this one, and
> it was not about element types at all.** This entry reads as *"`&(…)` works for scalars,
> and the open half is heap elements"*. Measured across POSITIONS instead of element types,
> the scalar half only worked at a PARAMETER: at a local, neither `b = &a` nor
> `b: &(integer, integer) = a` linked anything, at any element type. The first dropped the
> `&` and bound a copy — silently, on both backends — and the second typed a reference over a
> value, which the interpreter read as a store index and `--native` refused with a raw rustc
> `E0308` handed to the user. A `&(boolean, boolean)` local answered the un-swapped tuple with
> exit code 0. Fixed the same day (tuples.md D-tup-2, guard
> `tests/scripts/reference-tuple-local-binding.loft`): a tuple local is stack-backed, so it
> joins the scalars at `OpCreateStack` and B-Ref-Alias holds at every position for every
> admitted element type.
>
> **What stays open here is exactly the heap-element half**, and the table above is why. The
> entry's own framing — element types — is what hid a whole axis: a rule quantified over "ANY
> binding" is falsified by a POSITION as readily as by a type, and only one of those two was
> being swept.

> **D-bind-10 — CLOSED (2026-08-09) — the ⚑ VITAL rule was enforced for HALF of each
> expression.** The rule named `x + &y` as a parse error and grammar.md's D-gram-4
> declared the positional rule "total". Measured, four shapes compiled on both backends:
>
> ```loft
> b = 1 + &a;                         // an operand — the rule's OWN named example
> b += &a;                            // a compound-assignment RHS: not a bind site
> fn g(a: integer) -> integer { 1 + &a }   // a block-final tail value
> s = S { x: &a };                    // a struct-literal field of a NON-reference type
> ```
>
> **The mechanism, and why the sweep had to be over positions.** The guard
> (`operators.rs::parse_operators`, deepest precedence level) decided by peeking the token
> AFTER the `&`-operand: a `;` or `}` there meant "the `&` was the whole RHS". That proves
> nothing FOLLOWS the `&` — never that nothing PRECEDED it — so every shape where the `&`
> is the LAST operand of a larger expression passed. The one sub-expression test,
> `pln87_amp_in_subexpr_is_error`, puts the `&` at the HEAD (`b = &a + 1`), which is the
> single sub-expression position that peek did catch. One cell, one direction.
>
> **The fix supplies the other half.** The first primary of a binding RHS consumes an
> `amp_head` marker; a `&` reached after any operator, or inside a nested construct, sees
> it gone. The accept condition is now `terminates AND at head` — the pair is total. The
> head is opened in exactly three places: a plain `=` RHS (`parse_assign_op`), a statement
> start (so a bare `&a;` still reaches D-bind-7's own message), and a `reference<τ>` field
> value (`B-Ref-StoredRef`). Emitted IR + native Rust are byte-identical over the
> eight-shape accept corpus.
>
> **What the position sweep still missed, and the axis it held fixed.** The first fix
> rejected `S { x: &a }` for every field type — and broke `store_compact_b2.loft`, where
> `Linked { link: &pool[i] }` fills a `reference<Leaf>` field. Legality there is decided by
> the field's TYPE, not by the `&`'s position, and a sweep that varies position while
> pinning the type reads as complete and is not. That is `B-Ref-StoredRef`, previously
> unstated anywhere in this doc.
>
> A pre-freeze error-add (`manifest::CONTRACT_VERSION == 0`; [COMPATIBILITY.md § The error
> surface is one-directional](../COMPATIBILITY.md)) — every program it rejects was already
> silently dropping the `&` and binding a copy. Lock-ins: `pln87_amp_as_tail_operand_*`,
> `pln87_amp_as_compound_assign_rhs_*`, `pln87_amp_in_block_final_expression_*`,
> `pln87_amp_in_struct_literal_field_*`, `pln87_amp_in_return_statement_*` in
> `tests/parse_errors.rs`, with the ACCEPT half — every legal `&` position, each asserting
> the write reaches the source — in `tests/scripts/150-amp-head-position.loft`. The @PLN87 ladder (L1–L6), the model + doc reconciliation (PR#436), the residual
D-bind-7 and D-bind-8 (closed below) are all verified; @PLN40's Const-Bind / Const-Value /
Const-ScalarCollapse / Const-Compose are shipped and enforced for struct fields, parameters,
and locals — and, since @PLN102 K1, for **enum-variant fields** too (their one former residual
gap, D-const-1, now closed).

> **D-bind-9 — CLOSED same day (2026-08-05).** B-Ref-Reshape landed from the maker's sentence,
> which named REMOVAL, so the other two of `B-Disturb`'s events kept silently downgrading a `&`
> to a copy — measured on both backends, each with a *"copied out of"* advice line:
>
> ```loft
> c = &s[30];  c.key = 5;                                     // RE-KEY: s[5] was ABSENT
> c = &bx.inner;  bx = Mid { inner: Box { n: 22 } };  c.n = 99; // REASSIGN: bx.inner.n was 22
> ```
>
> Closing D-bind-8 while these held was an accounting error: the deviation named all three
> mechanisms of one rule and the sign-off covered one. Both now refuse, under the C79 principle
> (*decline what we cannot implement safely*) rather than as a second special case. The
> reassignment arm is the same liveness walk with the cause filter dropped; the re-key arm
> refuses at `note_key_field_write` where the base `is_amp_link`, and needs no liveness question
> because the key write IS the use. Lock-ins `b_ref_reshape_rekey_through_amp_link_is_error` and
> `b_ref_reshape_container_reassign_under_amp_link_is_error`, with their positive twins (a
> NON-key field still writes through; a reference dead before the reassignment still does).
>
> **The sweep that found them is the reusable part:** 14 shapes of `&` — whole struct, whole
> vector, element, field, nested, keyed non-key, keyed key, local reassign, callee reassign,
> `&` param mutate, `&` param rebind, loop, branch, overwrite-in-place — each asserting the one
> thing `&` promises, that the write reaches the source. Twelve honoured it; these two did not.
> A rule with more than one producer needs a sweep, not a cell.

> **D-bind-8 — CLOSED by adding B-Ref-Reshape (@PLN130 F9, [loft#779](https://github.com/loft-lang/loft/issues/779)).**
>
> B-Ref-Alias is unconditional — *"the `&τ` annotation makes ANY binding a live LINK to the
> source"* — and the code had one exception: three shapes where the write was silently
> DISCARDED, on both backends.
>
> ```loft
> // (a) the element does not even MOVE: `remove(2)` drops the last element.
> c = &v[0];  v.remove(2);  c.n = 99;      // v[0].n was 11 — the write was discarded.
> // (b) the element moves (index 2 -> 1) and the link does not follow it.
> c = &v[2];  v.remove(0);  c.n = 99;      // v[1].n was 33.
> // (c) the reshape is in the CALLEE, through a `&` parameter — and here with NO diagnostic.
> fn shift(target: &Box, all: &vector<Box>) { all.remove(0); target.n = 99; }
> shift(v[2], v);
> ```
>
> **The resolution is REFUSAL, not repair** (maker, 2026-08-05: *"The removal of anything from
> a structure (vector for example) that has an open `&` relation (for us an edge case) should
> be forbidden on compile time"*), so all three are now compile-time errors under the new rule
> **B-Ref-Reshape** above. That makes the pair total with no runtime machinery: a `&` always
> writes through, because the one shape where it could not is rejected before it runs. It is
> the rustc bargain in loft's spelling — rustc refuses the mutation while a borrow lives, loft
> refuses the removal — and it is affordable precisely because the maker classes it an edge
> case. Following the link instead (`if link.pos > removed.pos { link.pos -= size }`, which a
> dense vector makes arithmetic rather than a lookup) was feasible and was declined: not worth
> per-link runtime arithmetic for an edge case.
>
> A pre-freeze error-add. `manifest::CONTRACT_VERSION == 0`, and
> [COMPATIBILITY.md § The error surface is one-directional](../COMPATIBILITY.md) says loft may
> always DROP an error and after the freeze may never ADD one — so every place loft is too
> permissive is a last-chance-to-add, and every program this rejects was already silently
> wrong.
>
> **Two things measurement changed about the shape of the fix**, both recorded in
> `probes/40-reshape-refusal/README.md`:
>
> - the cross-frame half does **not** key on the `&` token. A plain struct PARAMETER aliases
>   the caller's element exactly as a `&` one does (cell X9: `fn w(t: Box) { t.n = 99 }` called
>   as `w(v[2])` writes 99 into the caller's `v`), and loft's own `warn_redundant_amp` advice
>   tells authors so — refusing only the `&` spelling would mean taking that advice trades a
>   compile error for a silent lost write. loft#779's own table asserted the opposite (*"plain
>   param copies (C86), so nothing to lose"*); that row is measurement-contradicted;
> - a plain LOCAL bind stays exempt for the opposite and equally measured reason: it does not
>   alias across a reshape, because @PLN130 F2 materialises it and says so.
>
> **Why D-bind-4 did not catch it.** Its lock-in is `c=&v[0]; c=9; v[0]==9` — no reshape. The
> rule was stated unconditionally and verified only in the simple shape, so a later change could
> narrow it without flipping any cell. The conformance lock-ins are now
> `b_ref_reshape_*` in `tests/parse_errors.rs` (six refused shapes and three positive ones) plus
> `tests/scripts/149-reference-survives-callee-reshape.loft`; `tests/scripts/145-…` and `774-…`
> pin the PLAIN-bind behaviour, which is unchanged.
>
> **What it did NOT close:** the other two disturbances. The maker's sentence named removal, so
> the RE-KEY and REASSIGNMENT causes were scoped out and still downgrade a `&` to a copy — now
> tracked as **D-bind-9** above, under the widened C79 principle rather than as an open question.

> **Landed via @PLN102 K1 (verified, closed):**
> - **D-const-1 — enum-variant `const` / value-const fields are now enforced identically
>   to struct fields.**  `enum Shape { Circle { const radius: integer }, … }`; after
>   `if s is Circle { … }`, the direct write `s.radius = 9` is now REJECTED at parse time
>   (backend-independent, so no interp/native split). Root cause was that the field-write
>   guard resolved the field table via `Parts::Struct(fields)` only; the fix extends BOTH
>   the leaf-field block (`validate_write`) and the value-const chain-walk
>   (`lhs_frozen_through`) to also walk `Parts::EnumValue(_, fields)` — the variant def's
>   `attributes()[f_nr]` aligns with its `EnumValue` field order, so the const_field /
>   value_const checks apply unchanged (verified: the positive cells stay accepted, no
>   over-reach into a pattern-bound local copy). Diagnostics now name the owner as a
>   "variant". A pre-freeze error-add (`CONTRACT_VERSION` was 0). Regression: the boundary
>   matrix graduated to `pln40_enum_variant_*` in `tests/issues.rs` (negatives + the
>   over-reach guard) and the positive cells in `tests/scripts/40-const-fields.loft`, both
>   backends. The remaining laundering-via-local / -return / -generic scopes stay deferred
>   (Phase 3, post-1.0; see
>   [../plans/40-const-fields/const-model-phase2.md § Phase 3](../plans/40-const-fields/const-model-phase2.md)).

> **Landed via @PLN87 / PR#436 (verified, closed):**
> - **D-bind-0** — `&τ` is now `Type::RefVar` (a reference type the variable carries); `&` is
>   no longer a general operator (a dedicated diagnostic rejects it elsewhere). Reads/writes
>   dispatch on the variable's RefVar type, not a per-expression flag.
> - **D-bind-1 / D-bind-2 (NORTH STAR)** — scalar live read + write-through: `a=3; b=&a; b=4;
>   a==4` → verified on interp **and** native.
> - **D-bind-3** — struct-field reference write-through: `b=&s.x; b=4; s.x==4` (the #415 gate
>   no longer blocks it).
> - **D-bind-4** — vector-element reference: `c=&v[0]; c=9; v[0]==9`.
> - **D-bind-6** — `&`-parameter link: `fn f(b:&integer){b=4}; f(a); a==4` → both backends.
> - **D-bind-doc** — `OWNERSHIP_MODEL § The law` rewritten to "heap aliases by default; `&`
>   binds a live REFERENCE"; the write-back framing is gone.
> - **D-bind-7 (the last residual ⚑ vital position)** — a bare `&a;` statement (and a
>   block-final `{ &a }`, the same leak) is now parse-rejected. The fix sits at the statement
>   chokepoint, `parser/expressions.rs::parse_assign`: a statement that BEGAN with a prefix
>   `&` whose `&` was not consumed by an assignment is the non-binding use the rule forbids.
>   The `operators.rs` guard clears `amp_pending` whenever it has already reported the `&`
>   (sub-expression / non-place), so the flag is still set at the chokepoint only in the
>   unreported bare/block-final case; a `started_with_amp` gate keeps a leaked flag from a
>   nested `&(…)` parse from mis-firing. Verified on interp **and** native; `pln87_d_bind_7_*`
>   in `tests/parse_errors.rs` (bare statement · bare field statement · block-final). The
>   caret points at the `&`.
>
> The former deferred case has **landed**: `&`-write-back from a CALL/var RHS
> (`fn f(o: &Obj){ o = mk() }`) now routes the RHS through a transferable owned temp, so the
> write-back reaches the caller (`a.x == 9`) — verified on interp **and** native. The parse
> rejection is gone; `tests/issues.rs::pln87_amp_writeback_from_call_writes_back` is an active,
> passing test (no longer `#[ignore]`d).

## Carried by binding.md until 2026-09-04

The rules doc used to carry these beside its `OPEN` line — closure summaries, and notes on
the times the count read 0 over a live entry.  They are timeline, so they moved here
unchanged; [binding.md](binding.md) now states only what is open.

### D-bind-17 — OPEN (2026-09-05, loft#1372, @PLN153 phase 4): a `&` link to a nullable slot is declined

`(B-Ref-Intro)` admits `&τ` for every τ; `(B-Ref-Write)` and `(B-Ref-Uniform)` make a `&τ`
variable a τ variable that writes the source; `(F-ParamRef)` makes a `&` parameter the
write-back channel.  Nothing restricts τ, so `&integer?` should link a nullable slot.  It does
not: a link's inner type is asked BARE at every read and write site — `??` sees `&integer?`
and refuses its default, `+` and a copy-out see a non-null slot and warn, the interpreter's
write dispatch has no arm for the wrapper, native's parameter type does not match the write —
and the `&` of a nullable LOCAL fell past the scalar and record arms of the lowering and bound
a silent COPY: `q = &x; q = 7` left `x: integer?` at 5 on both backends, with nothing said.
The parameter's body was refused on its write and on its read with retype messages that named
neither the link nor a cure.

Found by @PLN153 phase 4's `optional` screen: the lowering's `is_scalar` closure and record
test were two of the 353 bare shape tests, and the cell built to reach them was the finding.
Lifting the refusals and peeling those two tests showed the whole read/write family behind
them (`stage4/amp/*.loft`), which is the FEATURE this entry records as owed.  Until it lands,
both spellings are DECLINED where the link type is built — `Parser::ref_var_type`, and the
field-link site beside it — with one message naming the cure (`@FR-B-Ref-Reshape`: a link
loft cannot honour is refused, never downgraded).  Guard:
`tests/scripts/153-a-link-to-a-nullable-slot-is-declined.loft` (five spellings, one message
each).  Closes when a `&τ?` binding reads and writes its slot as a `τ?` on both backends; the
shape to build is the chokepoint phase 3 gave `(N-Store)`: one spelling of "the slot behind a
link" every site asks through.  The whole-value write through a `&` link to a text, a struct
or a vector — the NON-nullable controls of the same matrix — is loft#1371, apart.

### D-bind-16 / D-bind-11 closure summaries

**D-bind-16 CLOSED 2026-09-03** (loft#1321): `(B-Copy)` is read ARM BY ARM at a branch join.
Every arm tail a plain bind would copy — a variable, a parameter, a loop element, a call whose
record return the caller must copy — is rewritten into the bound spelling on a temp
(`scopes.rs::lift_join_arm_tails`), bound by that plain bind's own lowering, and the join
borrows the temps; an arm a plain bind would view stays a view (`(B-View-Depth)`'s `vv[0] ??
[0]` is the control).  It is the copy face of ownership.md's D-own-8, closed by the same
change; the reverted attempt's three facts (a `??` hoists its subject; the join is not to be
read whole; a call arm may return a borrow) are all honoured by binding each arm through the
lowering that already knows them.  Guard:
`tests/scripts/1321-a-joined-binding-copies-what-a-plain-bind-copies.loft`, falsified at
26d17f4b on both backends.  ⚠ A branch consumed as a call ARGUMENT is NOT copied: an argument
aliases (calls.md F-ParamHeap).  ⚠ And a binding the join is not the one assignment of keeps
the runtime join bind it had (`r = x; for … { r = v[i] ?? x }` copies through
`OpBindOrCopy`): one variable carries one type-level fact for all its Sets.

**D-bind-11 CLOSED 2026-09-03** (loft#1006): a `&(τ, …)` holds any element a struct field can.
The two representations the register named were never a choice — the stack form cannot own a
`text` element, and the record form already did this exact swap through the loop variable —
so a `&(…)` whose elements are not all scalars is a reference to the `__tuple<…>` record, a
linked tuple LOCAL is built as that record, and the rule is written down as `(T-Ref-Rep)` in
[tuples.md](tuples.md).  A nullable, fn-ref or nested-tuple element stays refused and the
refusal names it.

### the status line formal/README.md's area table carried until 2026-09-04

**0 open** — D-bind-11 CLOSED 2026-09-03 (a `&(τ, …)` with a heap element is a reference to the `__tuple<…>` record; it had read: `&(τ, …)` admits only SCALAR elements, against B-Ref-Alias/B-Ref-Uniform) and D-bind-16 CLOSED the same day (a join binds every arm a plain bind would copy through its own temp, loft#1321); (the D-bind-11 entry read: — the two backends represent a reference tuple differently and `text` is the first element where that shows; loft#1006) — **B-Ref-AnnotationOnly is now total** (D-bind-10, 2026-08-09): a `&` that was the LAST operand of an expression (`b = 1 + &a`, `b += &a`, a block-final `1 + &a`, `S { x: &a }`) used to compile, because the guard peeked only the token AFTER the operand; `B-Ref-StoredRef` records the one legal non-binding position, a `reference<τ>` field. **B-Ref-Reshape** landed (@PLN130 F9, loft#779): disturbing a container while a `&` reference into it is LIVE is a compile error, for all three of `B-Disturb`'s events (removal, re-key, container reassignment). It is the first application of C79's 2026-08-05 *decline-what-we-cannot-implement-safely* revisit, whose reason is forward compatibility: an error can be dropped later, a silently different semantics cannot. Also closed: `&` is a TYPE ANNOTATION (`&τ` = `Type::RefVar`), @PLN87 ladder L1–L6 + D-bind-7 closed; the @PLN40 two-level `const` model (Const-Bind/Value/…) shipped, and D-const-1 (enum-variant const) closed via @PLN102 K1 — enforced identically to struct fields, both backends

