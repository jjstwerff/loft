# formal/layout-history.md — the deviation register for [layout.md](layout.md)

> **The rules are next door.**  [layout.md](layout.md) states what must always be true of the
> language; this file is its TIMELINE — every place the code was measured not to do it, when,
> what it cost, and what closed it.  The two are apart because a contract a reader has to skim
> past its own history stops being a contract they can skim.  The rules doc carries the CURRENT
> state (how many are open, and which); everything below is the record behind it.

- **D-layout-1 — no version guard on persisted bytes** (the motivating gap). Before @PLN97 the
  layout was **nowhere written** and **nothing recorded which layout a store was written under**,
  so a layout-changing fix (#477: nested-vector stride 4→8) **silently misread** existing data —
  caught only by breakage, never by a check. `L-Sound` is the rule it violated.

  **Status — mechanism shipped, auto-enforcement pending a consumer.** The rule is now
  *enforceable*: the golden test (`tests/layout_golden.rs`) catches a layout change at commit time,
  and the `.dschema` sidecar (`src/schema_sidecar.rs`, `CorruptReason::SchemaMismatch`) detects a
  stale store at load and routes it through the durable store's `on_corruption` rebuild. **Residual:**
  the durable store (`plans/43`) is not yet driven by a loft builtin, so nothing *calls* the
  load-time gate automatically yet — the deviation closes fully when a persistence consumer wires
  `check_beside` into its open path. Until then the guard exists but is opt-in.

- **D-layout-2 — the `?` changed the layout** (2026-08-28, loft#1125). `L-Null` says
  `layout(τ) = layout(τ?)`, and three sites decided layout by naming `Type` variants BARE, so a
  wrapped shape reached none of them.

  The visible one: the walk that gives an `index` its bookkeeping triple a position runs at the
  end of `fill_all` precisely so `#left_N / #right_N / #color_N` are appended to the ELEMENT
  struct before it is sized. `Optional(Index(…))` matched nothing there, the triple was appended
  afterwards, and `finish_type` returns early for a type that already has a size — so all three
  kept `position: 0`, on top of each other and of the first real field. The nullable form then
  refused to lay out at all while its dense twin was fine.

  Its two siblings, same rule: the `null` → sentinel conversion asked for `Type::Vector` alone
  and was short by the five KEYED kinds and by the wrapper, so a `spatial<P[x,y]>? = null` local
  kept a bare `Value::Null` — which writes nothing — and the scope-exit `OpFreeRef` read the
  untouched bytes as store #0 (BUG #306); and an OMITTED nullable collection FIELD took the zero
  its type gives, where zero is the EMPTY collection and absence has its own reserved id
  (`DbRef::ABSENT_REC`, loft#917), so `c: vector<τ>? = null` read back present-and-empty.

  **Status — CLOSED.** All three read through `base()`. Guard:
  `tests/scripts/a-nullable-collection-lays-out-like-its-dense-twin.loft`, which gives every
  keyed kind its OWN element struct: the layout half is invisible whenever the same index type
  also has a dense local somewhere in the program, because that one registers the bookkeeping in
  time and the nullable form inherits a correct layout.

- **D-layout-3 — three writers did not go through the tag** (2026-08-30, loft#1198). `L-Null-Tag`
  ends *"every writer and reader of such a slot goes through the tag; the pair that holds this is
  `emit_nullable_slot_write` / `emit_nullable_slot_read`"*, and the sentence was a description of
  one writer out of four. Deciding to tag needs the SOURCE's type, and a nullable struct has two
  spellings that mean one thing — the dense `S` and the `S?` a function returns or a local
  declares. The tuple's writer asked `needs_nullable_wrap`, which reads both. The struct field
  (`objects.rs::handle_field`), the element store (`collections.rs`) and the append
  (`vectors.rs`) each spelled `let Type::Reference(src_d, _) = src_tp` instead and so could see
  only the dense one.

  For every `S?`-spelled source the dense record therefore went in untagged, which is `L-Null`'s
  layout applied where `L-Null-Tag` governs — the same confusion of the two halves that D-tup-6
  and D-layout-2 are, arriving this time from the WRITE side. Two faces: a present value landed
  one field low so every read came back one field high, and a value the callee withheld at
  runtime wrote nothing at all, leaving the slot reading PRESENT with its previous value. With
  the discriminant aliased onto the payload's first field, `S { a: 0, … }` read back ABSENT.

  **Status — CLOSED.** All three route through `emit_nullable_slot_write`, which now also
  releases the payload the slot held on its PRESENT arm — one of the three carried that free and
  the shared home did not, so absorbing them without it would have traded a wrong answer for a
  leak. Guard: `tests/scripts/1198-a-nullable-source-is-tagged-into-its-slot.loft`, whose
  controls are the dense source (the half a corpus of literals can see) and the tuple member
  (the writer that already obeyed the rule).

- **D-layout-4 — `τ?` picked the WRONG HALF of the split for a pointer field** (2026-09-03,
  loft#1316). `L-Null` and `L-Null-Tag` divide on a property of the type: a τ that reserves a
  null VALUE keeps its own bytes and spends the reserved pattern, and only a struct stored INLINE
  needs the tag. A stored reference reserves `nullref`, so `reference<T>?` is `L-Null`'s case and
  must lay out as the 12-byte pointer it already is. The rewrite that mints `__nullable<S>` gave
  it `L-Null-Tag`'s instead.

  Both notions travel as `Type::Reference`, told apart by the FIELD's own `u16::MAX` share marker
  (#328) — the same bit `has_value_cycle` reads to skip pointer edges. `synth_nullable_struct_fields`
  discarded the deps with `_` and so converted both. Measured: `reference<Leaf>?` and `Leaf?` laid
  out byte-identically (`__nullable<Leaf>[16/8]`) where `reference<Leaf>` is `dbref[12/4]`.

  Three faces, one mechanism, and the loudest hid the others:
  * **the layout error.** A struct stored inline cannot contain itself, so on a reference graph
    that returns to its own struct the field has no finite size: `struct Node { next:
    reference<Node>? }` failed with *"field 'next' has no position (u16::MAX)"*. The terminator of
    a linked list is the one slot that MUST hold null, and the only spelling that compiled was the
    non-null one — which is why loft#1313 had to suppress `(N-Store)` for exactly this shape
    rather than name a cure that does not compile.
  * **the refusal.** `&pool[i]` in a literal is admitted by the FIELD'S TYPE (`B-Ref-StoredRef`),
    and the gate matched `Type::Reference` unpeeled, so writing `?` withdrew the `&` the pointer
    field exists for.
  * **the quiet one.** `h.l = &pool[i]` fell past the `#328` repoint arm, also unpeeled, to
    `copy_ref` — a deep copy through the field's CURRENT value. On an acyclic type that compiles
    and answers plausibly: the control prints `11` where a pointer prints `22`, so declaring `?`
    silently replaced sharing with a copy, with no diagnostic anywhere.

  **Status — CLOSED.** All three read the marker or peel with `base()`. loft#1313's suppression
  (`field_has_no_nullable_spelling` + `Data::reference_cycle_back_to`) is deleted with it, and its
  guard cells in `tests/heap_nstore.rs` flip from silent to warning. The notice they now emit had
  its own defect of the same kind: `Type::name` renders a pointer field as the bare struct name,
  so the cure read `Node?` — the INLINE form, which on this shape does not compile, and on an
  acyclic one compiles while swapping the pointer for a copy. `Parser::cure_spelling` names the
  field's own type. Guard: `tests/scripts/1316-a-nullable-reference-field-is-still-a-pointer.loft`,
  whose controls are the `?`-less pointer field (still shares) and the embedded `T?` (still
  copies — `L-Null-Tag` still governs it).

  ⚠ **The third entry in a row for one mechanism, and each closed on "all three sites".**
  D-layout-2 was a nullable shape reaching a bare `Type` match, D-layout-3 was the same from the
  write side, and this is the same again with the added twist that the two halves of the split
  are BOTH reachable through one IR spelling. What repeats is not a site but a habit: a `Type`
  matched without `base()` and without the marker, in a position a field type reaches. A census
  of the parser plus `typedef.rs` finds **103** bare `Type::Reference` matches, of which **21**
  read a field's declared type. That number is a QUEUE, not a defect count — most read the
  synthetic `__nullable<S>`'s own payload attribute, where no `Optional` can arrive — but it is
  the population any next instance of this class will be drawn from, and it is the reason the
  rule-led walk over `@FR-L-Null` is not finished by this entry.
