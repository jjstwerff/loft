<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# formal/closures.md — semantics for lambdas, closures, and fn-refs (strict)

**Catalogue:** @F22 (closures & value capture), @F23 (function references), @PLN89 (oracle).

> **Rules then deviations** (see [README](README.md)). This is the relation for loft's
> **first-class functions**: the two lambda forms, closure **capture**, function references, and
> application. It extends [calls.md](calls.md) (application is a call) and [heap.md](heap.md) (a
> capturing closure's environment is a heap record). Unlike the other operational files, this one
> has **open deviations**: the two lambda forms differ in capture, and one combinator path
> crashes. The Rules below are the **intended contract** (what a user should be able to rely on);
> the Deviations are exactly where today's implementation falls short — written so they can be
> driven to zero.

## The two forms — pure syntactic sugar (both capture)

loft has two lambda syntaxes, and (since 2026-07-04, D-clo-1 closed) they are **pure syntactic
sugar for the same thing** — both capture outer variables identically. The only difference is
ergonomics:

| form | captures outer locals? | ergonomics |
|---|---|---|
| `fn(p: T, …) -> R { body }` | **yes** | explicit parameter + return types; use anywhere |
| `\|p, …\| { body }` / `\|\| { body }` | **yes** | parameter types INFERRED from context (a `map`/`filter` callee's element type); no `->` return annotation |

A bare function name (`f`, not `f()`) is a **function reference** — a first-class value of type
`fn(T…) -> R`, a closure with an empty environment.

## Notation

Uses [calls.md](calls.md)'s call relation and [heap.md](heap.md)'s heap `H`. A **closure** is a
pair `⟨code, env⟩`: the lambda body plus a captured environment (the outer variables it names). A
**fn-ref** is a closure with an empty environment.

---

## Rules

### Construction — a closure captures the outer variables it names

```
  (L-Fn)     fn(p₁…pₙ) -> R { body }  AND  |p₁…pₙ| { body } / || { body }   both evaluate to a
             closure ⟨body, env⟩ where env captures every OUTER variable the body references but
             does not bind.  The two forms are equivalent modulo type-annotation ergonomics.
  (L-FnRef)  a bare function name f (in a value position / fn-typed context) is a fn-ref value —
             a closure with an empty environment.
```

**In words.** Both `fn(y) -> integer { y + x }` and `|y| { y + x }` build a closure that
**captures** `x` from the surrounding scope — they are the same construct, differing only in that
the `|…|` form infers its parameter types from context (so it is the ergonomic `map`/`filter`
callback) while the `fn(){}` form spells them out (so it works where no type context is
available). A bare `f` (a function's name used as a value) is a first-class function reference.

### Capture semantics — scalar by value at creation, heap shared

```
  (L-CapScalar)  a captured SCALAR is captured BY VALUE at closure creation: the closure sees the
                 value the variable had when the closure was formed.
  (L-CapHeap)    a captured HEAP value (struct/vector) is SHARED: a mutation-through the source
                 AFTER capture is visible inside the closure (consistent with calls.md
                 F-ParamHeap — capture, like a call, shares heap state, copies scalars).
```

**In words.** A closure that captures an `integer x` freezes `x`'s value at the moment the closure
is built (verified: capture, then `x = 20`, still yields `10`). A closure that captures a struct or
vector shares it — mutating a field of the captured value afterwards shows up when the closure runs
(verified: `b.v = 9` after capture yields `9`). This mirrors the parameter contract in
[calls.md](calls.md): heap is shared, scalars are copied.

### First-class — store, pass, return; application is a call

```
  (L-Apply)   ⟨c(args), σ⟩   applies closure/fn-ref c: bind its parameters to args (calls.md
              F-Args/F-Param*), run body in ⟨code, env⟩, yield the return (calls.md F-Call).
  (L-Escape)  a closure is a VALUE: it may be stored in a variable or struct field, passed as an
              argument, and RETURNED from a function — a returned closure keeps its captures
              (it escapes cleanly).
```

**In words.** A closure is an ordinary value. You can put it in a variable or a struct field, pass
it to another function, and return it — and a returned closure still remembers what it captured
(verified: `fn mk(n) -> fn()->integer { fn()->integer { n } }`, then `mk(7)()` yields `7`; a
closure in a struct field `h.f()` yields `42`). Calling it is just a call ([calls.md](calls.md)),
with the closure's environment in scope.

---

## Deviations

OPEN: **3** — a lambda's `??`-default store discarded INLINE leaks one store per call
(D-clo-7, below; that entry's value half and its BOUND-return leak half are both closed);
D-clo-12, a forwarding function's return type cannot carry what its fn-ref ARGUMENT knows, so
it frees the capture (loft#1185); and D-clo-13, a lambda whose tail JOINS a capture with a
mint has one dep list for two ownerships and no reading serves both arms (loft#1186).
D-clo-11 — a captured STRUCT taken by the caller's bind — D-clo-10 — a captured collection
taken the same way — D-clo-9 — a captured record FREED by a caller that lifted a fn-ref tail
— and D-clo-8 — a captured `vector<(…)>` unpacked rather than shared — were opened and closed
on 2026-08-29, 2026-08-29, 2026-08-29 and 2026-08-28. Closed: both
lambda forms capture identically (D-clo-1), the
stored-short-lambda combinator crash is now a clean diagnostic (D-clo-2) — both closed
2026-07-04 —
`L-Escape`'s *storage* half is complete (D-clo-3, opened and closed 2026-08-22), a lambda
now carries one text work buffer however many promotions ask for one (D-clo-4), a
combinator's inline callback is handed the buffer its ABI expects (D-clo-5), and a fn-ref
call carries every text buffer its target declares (D-clo-6) — all three opened and closed
2026-08-27.

⚠ This zero is only as strong as the axes the corpus below varies, and it has now been
re-measured TWICE and broken both times. D-clo-3 found the *first-Set vs re-Set* axis;
D-clo-4 found the axis inside the BODY — every `L-Apply` cell returned through a single
delivery, so nothing varied *how many* promotions the body asks for. The axes now varied
are destination (local, struct field, vector element, tuple member, return) ×
first-Set/re-Set × source (bare name, non-capturing lambda, capturing lambda, local, call,
`if`/`match` arm) × host (named local, `&` parameter, vector element, field chain) ×
**buffer count asked for by the body (one, two)**. Two that remain HELD FIXED, and are
therefore where a next re-measurement should look: the number of DISTINCT capturing
lambdas per attribute (one, by a shipped rule), and the nesting of the holder itself (a
struct that holds a capturing closure cannot go in a collection at all, by #318).

A THIRD axis was held fixed and is now varied: WHERE the lambda is applied. Every
`L-Apply` cell called the closure directly, and a combinator lowers its own call — so a
capturing lambda passed INLINE to `map` and returning text faulted on `--interpret` while
`--native` ran it. That is D-clo-5, closed the same day.

> **The re-measurement, and what the corpus was holding fixed (2026-08-22).** The
> Conformance section below verifies `L-Escape` at three destinations — a local, a struct
> field, and a return — and every one of them writes into a place being **initialised**.
> The axis it never varied is therefore not the container at all but *first-Set vs re-Set*,
> and on that axis a live crash was sitting under the zero: a fn-ref written by a
> NON-CAPTURING source (a bare name, or a lambda capturing nothing) lowers to the 8-byte
> d_nr while the slot is the 20-byte pair, and only the initialising paths topped it up.
> `g = inc` on a live `g` panicked `fn_call_ref: fn_var=16 < 20` on `--interpret` while
> `--native` ran the same program — a backend SPLIT, so neither backend alone could see it
> — and `t.0 = inc` panicked on one backend and handed the user a raw rustc E0308 on the
> other. Fixed at the three destination-aware sites (`set_var`, the `TuplePut` arms of both
> backends, and the native reachability walk), guarded by
> `tests/scripts/fn-ref-reassignment-tops-up-the-pair.loft`, which was confirmed to fail on
> a pristine tree on both backends.
>
> The rest of the destination sweep came back clean and is recorded here so it is not
> re-run: vector element (literal and `+= [f]`), keyed-collection value, struct-enum
> variant payload read per-variant, nested struct-in-vector, and an un-inferrable stored
> short lambda through `map`/`any`/`all`/`sort_by`/`filter` (D-clo-2's fix named
> `parse_map` alone, but the diagnostic fires at the LAMBDA, so it was never the
> single-site risk it looked like).

> **D-clo-15 — OPENED AND CLOSED (2026-08-29, loft#1178): a declared-collection lambda whose
> tail pass 2 REPLACES aborted the compiler.** `(L-Escape)` says a closure is an ordinary
> value that may be stored, passed and returned, and `(L-Apply)` that calling one is a call;
> `g = fn(v: integer) -> vector<integer> { xs = [1, 2]; xs.map(…) }` is both, and it did not
> compile at all — `H5 two-pass contract: grew a pass-2-only attribute __vdb_2`.
>
> The reservation was read off the PASS-1 tail, and this body defeats that read outright: its
> pass-1 tail is `Var(xs)`, a named local that already owns a store — the exact spelling of
> the bodies that must NOT get a buffer — while pass 2 lowers the `map` into a fresh one. The
> two passes are not looking at the same tail, so no predicate over the pass-1 one can
> separate the rows. Reserving for EVERY declared-collection lambda is what compiles them
> all, and the two things that blocked it are now closed: `State::fn_return` releases the
> buffer a callee did not hand back (D-clo-7's fix) and the native dispatch now asks the same
> question of the VALUE that came back rather than of the deps that declared an intent. The
> one exception is a fact rather than a prediction — a CAPTURE tail has nothing to deliver
> (D-clo-14).
>
> Two defects had to come out of the way, and each is its own sentence:
>
> - a lambda nested in another lambda's body left `last_closure_work_var` set, so the OUTER
>   fn-ref was mapped to a closure variable living in the INNER lambda's table and `--native`
>   emitted `var_??` for it. The named-function reset states the same rule one scope out
>   (*"a lambda inside make_adder leaks last_closure_work_var into the next function
>   parsed"*); a lambda inside a lambda is that leak within one body.
> - `--native` could not compile the map row: the desugar's `_map_result_1` is built INSIDE
>   the comprehension block and handed back from outside it, and a Rust `let` lives where the
>   emission first reaches it. The interpreter cannot have that — a local is a frame slot
>   wherever it is written — so it is a property of the EMISSION. Every VIEW a `return` names
>   is now bound up front, the cure loft#731 gave the iteration scratch for the identical
>   error. A view only: #354 measured the other half, and hoisting a heap local that OWNS its
>   store re-inits a fresh one per call that the matched free no longer covers.
>
> Guard: `tests/scripts/1178-a-declared-collection-lambda-gets-its-buffer.loft`, which carries
> all seven rows of the issue's table because the reservation is now unconditional and the
> rows that must NOT fill a buffer are what says the runtime free carries them.

> **D-clo-14 — OPENED AND CLOSED (2026-08-29, loft#1182): a lambda handing back a place read
> out of a CAPTURE reserved a return buffer it then ignored.** `(L-CapHeap)` says the captured
> store belongs to the frame that made it, so there is nothing for the callee to place — and
> `ref_return`'s ladder had no verdict for that and fell through to `Grow`, so
> `fn(v: integer) -> vector<integer> { q.xs }` grew a hidden `q` buffer the body never fills.
>
> The two backends then disagreed, and that disagreement is the entry. `--interpret` was clean
> because `State::fn_return` releases any buffer the callee did not hand back (D-clo-7's fix),
> a RUNTIME check that does not care what the deps claim. `--native` reads the deps:
> `arm_frees_buf` frees an unfilled `__vc_hbuf` only when the candidate's return deps do NOT
> name a hidden heap attr, and they do, because the buffer exists. One store leaked per call.
>
> `classify_text_dep` has answered this exact question since @PLN85 — `TextDep::SkipCaptured`,
> *"captured closure var — read from the closure record; never promoted"*. One notion, two
> ladders, and only the text one could see it. The ref ladder now carries the same verdict.
>
> Guard: `tests/scripts/1182-a-captured-place-tail-reserves-no-buffer.loft`, whose native row
> moves on the leak channel and whose interpret row is INERT — a backend divergence can only
> move one, which is why `make falsify`'s conservative AND reports NOT falsified for it.

> **D-clo-11 — OPENED AND CLOSED (2026-08-29, loft#1181): a captured STRUCT was TAKEN by the
> caller's bind, and the same dep was dropped TWICE on its way to the call site.**
> `(L-CapHeap)` names struct and vector in one breath, so D-clo-10's *"only for a COLLECTION
> return"* was never a rule — it was where the measurement stopped. `r = s(1)` on
> `s = fn(v: integer) -> P { cap }` adopted the captured record and the rebind released it;
> `LOFT_STRICT_STORES=1` reported the use-after-free, and a SECOND capturing lambda in the
> same function turned it into a wrong answer by landing its closure record on the freed slot.
>
> That entry's stated reason — *"a struct return is MATERIALISED into a fresh copy before it
> leaves the callee"* — is false, and the IR says so in one line: the lambda's body is
> `return OpGetDbRef(__closure, 0)`. Nothing copies.
>
> Two independent drops, and the issue's two recorded repair attempts each failed on the
> other one:
>
> - **the fn-ref VARIABLE kept pass 1's type.** Pass 1 has not parsed the body, so the type
>   it publishes says the result is owned; pass 2 knows better. `is_equal` collapses deps, so
>   `change_var_type`'s equality early-return kept the uninformed answer and the call site
>   never saw the dep at all. A fn-ref slot now ADOPTS a refined return dep, for the reason
>   the `#663` element width beside it is adopted — same base type, so the frame the two
>   passes lay out is unchanged.
> - **`fnref_result_type` read *"an index naming no visible argument"* as the closure.** True
>   of `__closure` and false of `ref_return`'s `__ref_N`, and BOTH are out of range: `{ cap }`
>   borrows and `{ sr_make(k) }` owns, spelled identically. D-clo-10 recorded that as
>   *"no dep-index test can separate them"*, and that is true of a RANGE test and false of a
>   NAME test — `Argument::hidden` already carries the distinction and its own doc already
>   states the conclusion (*"should be excluded from dep propagation"*). A lambda now
>   publishes a return type whose leftover out-of-range index can only be the closure, which
>   is what lets the borrow be read without over-approximating the mint into a leak.
>
> ⚠ **The closure is read only where the lambda's tail is a PLACE** — a slot, or a field /
> element / capture read out of one. A tail that JOINS hands back the capture on one arm and
> a fresh store on the other while carrying ONE dep list, and neither reading is right twice:
> as a borrow the minting arm leaks four stores, as owned the capture arm is released while
> its variable is live. That is D-clo-13, and the restriction is what keeps this entry from
> trading one defect for the other.
>
> Guard: `tests/scripts/1181-a-captured-struct-is-not-the-callers-to-take.loft`, whose
> falsification row moves on ONE cell — the over-free is silent until something reuses the
> slot, so the direct-rebind cells pass on the control build and are scored by
> `LOFT_STRICT_STORES=1` instead.

> **D-clo-12 — OPEN (2026-08-29, loft#1185): a FORWARDING function frees the capture its
> fn-ref argument handed back.** `fn call_it(f: fn(integer) -> P, v: integer) -> P { f(v) }`
> called with a capture-returning lambda releases the captured record on the caller's rebind.
> Inside `call_it` the slot is a PARAMETER with a DECLARED fn-type, which carries no deps
> whatever closure is passed, so the closure read D-clo-11 installed is inert one frame down
> — the same predicate seen from the other side that D-clo-9 measured for monomorphs.
> D-clo-9 resolved it at the CALL SITE, where the caller named the closure it passed; here
> the forwarding function's return type is computed ONCE for every caller, so the fact has to
> travel differently: a return dep parametric in the fn-ref argument, or a per-argument
> re-derivation at the call site.

> **D-clo-13 — OPEN (2026-08-29, loft#1186): a lambda whose tail JOINS a capture with a mint
> has one dep list for two ownerships.** `fn(n: integer) -> P { cap ?? P { v: -1 } }` hands
> back the captured record when the subject is present and the call site's own return buffer
> when it is absent — and `State::fn_return` KEEPS that buffer (D-clo-7's fix, identified by
> store), so the caller owns it. Read as owned, the present arm is a use-after-free; read as
> a borrow, the absent arm leaks one store per call. The NAMED twin is clean on BOTH arms and
> says what the cure is: a direct call site mints the return buffer as a caller LOCAL that
> scope exit frees, so whichever arm runs the buffer has an owner. The fn-ref path has no
> such local. The cure is the symmetric twin of `push_fnref_text_buffers` — a fn-ref call site
> that may receive a heap delivery owns that buffer the way it already owns its `&text` ones,
> with `Data::fnref_text_buffers`' widest-candidate-then-trim shape as the precedent for the
> adaptive ABI.

> **D-clo-10 — OPENED AND CLOSED (2026-08-29, loft#1180): a captured COLLECTION was TAKEN by
> the caller's bind.** `(L-CapHeap)` says a captured heap value is SHARED — the caller may read
> it, never take it. `r = g(7)` on `g = fn(v: integer) -> vector<integer> { cap }` adopted the
> store and released it at scope exit, so `cap` answered EMPTY from the second call onward, on
> both backends, with nothing saying so.
>
> `fnref_result_type` maps a fn-ref call's return deps through the caller's actual arguments
> and DROPPED any index naming no visible one, on the stated grounds that *"the adaptive fn-ref
> ABI allocates those buffers at runtime, so the value arrives OWNED"*. That is true of a
> hidden work buffer and false of `__closure`, which is the CALLER's own record — D-clo-7's
> sentence one more time, *a dep dropped as uninteresting is not a dep that was never there*,
> in a third position after the `??`-lift (loft#1114) and the fn-ref tail (loft#1176). The
> dropped index now becomes a dep on the fn-ref VARIABLE, which is where the caller reaches
> its closure.
>
> Two restrictions, both measurements rather than caution:
>
> - only for a CAPTURING slot, read off the fn-ref TYPE's own deps. That predicate means what
>   it says HERE, where the slot is a caller local whose type was INFERRED at the bind; it is
>   inert one frame down, where the same slot is a parameter with a DECLARED fn-type
>   (loft#1176 measured that, and the two entries are the same predicate seen from both sides).
> - only for a COLLECTION return. ⚠ **Both halves of this restriction were wrong, and D-clo-11
>   closed it a few hours later.** A struct return is NOT materialised into a fresh copy —
>   the lambda's body is `return OpGetDbRef(__closure, 0)` and nothing copies — so
>   `fn(i: integer) -> P { cap }` was a use-after-free, not a value that "was always right".
>   And *"no dep-index test can separate them"* is true of a RANGE test only: the
>   out-of-range index is `__closure` for `{ cap }` and `__ref_N` for `{ sr_make(k) }`, and
>   `Argument::hidden` tells them apart by NAME. The leak this restriction was avoiding —
>   eleven stores in `717-closure-struct-return.loft` — is real and is what the name test
>   removes.
>
> ⚠ The captured-FIELD spelling (`{ q.xs }`) answers correctly now and still LEAKS one store
> per call on `--native` — loft#1182, a different mechanism: `ref_return` promotes the borrowed
> local into the return attribute, so the callee declares it delivers through a buffer it then
> ignores. The INLINE spelling was correct throughout, which is why this was first filed as a
> leak — nothing binds the result, so nothing adopts it.
>
> Guard: `tests/scripts/1180-a-captured-collection-is-not-the-callers-to-take.loft`.

> **D-clo-9 — OPENED AND CLOSED (2026-08-29, loft#1176): a captured record was FREED by a
> caller that lifted a fn-ref tail.** `(L-CapHeap)` says a captured heap value is SHARED, and
> a value the outer scope still names cannot be released by somebody else's scope exit.
>
> `fn once(x: P, f: fn(P) -> P) -> P { f(x) }` hands back a fresh store, the caller's own
> argument, or a record the closure CAPTURED, and its `-> P` reads the same in all three.
> The caller decided from `returns_borrowed_view`, the DEPS proxy: a capture-returning
> lambda's return dep names the hidden `__closure` attribute, and a hidden attr reads as
> *"not a borrow"*. So the caller lifted the result and freed it — the captured record
> answered another value on the next iteration and garbage once the scope ended, on BOTH
> backends. This is D-clo-7's licence exactly (*"a dep dropped as uninteresting is not a dep
> that was never there"*), in the direct-`Call` position rather than the `??` one that entry
> closed, and the `__retbuf` exemption made it worse: `{ f(x) }` never delivers INTO that
> buffer, so the premise that the lifted temp is the caller's own allocation is false there.
>
> The mirror image was live at the same time and is what the issue was filed for: the
> GENERIC spelling of the same source under-lifted, because the freshness proof it uses is
> read off the monomorph's body and a fn-ref's callee is a runtime value there — one leaked
> record per inline call. **One resolution answers both.** The callee's fact is unreachable
> from inside the callee and reachable at the CALL SITE, where the caller named the closure
> it passed: `fnref_target` resolves the definition and its own body-shaped freshness proof
> decides. Both ownership reads are needed and neither is redundant — the deps proxy catches
> a lambda handing back its own PARAMETER, the body proof catches one handing back a CAPTURE.
> An unresolved or ambiguous slot declines, which costs the leak that was already there.
>
> ⚠ **The fn-ref must be a caller LOCAL.** `fnref_target` maps variable slots, so one held in
> a struct field (`once(P { n: 41 }, h.f)`) resolves to nothing and declines — one leaked
> record per call, unchanged by this fix and recorded here rather than left implicit.
>
> Guard: `tests/scripts/1176-a-monomorph-whose-tail-is-a-fn-ref-call.loft`, whose two halves
> fail on DIFFERENT channels (the over-lift on an assertion, the under-lift on the exit leak)
> and whose header says which of them the falsification row can and cannot score.

> **D-clo-8 — OPENED AND CLOSED (2026-08-28, loft#1131): a captured `vector<(…)>` was
> UNPACKED instead of shared.** `(L-CapHeap)` says a captured heap value is SHARED, and the
> mechanism is a 12-byte `DbRef` in the closure record. `closure_attr_type` types every
> collection capture as `Reference(<element def>)` carrying the #328 share marker — the def
> is a stand-in for *"some DbRef"*, not a claim about what the slot holds.
>
> For a `vector<(…)>` that stand-in def is `__tuple<…>`, which is exactly what loft#821's
> per-element tuple write in `set_field_check` matches on. It read the DESTINATION slot's
> spelling while its own comment says the arm must be chosen by *"the SOURCE's
> representation rather than by which spelling the slot happened to carry"* — so the capture
> emitted the vector's own bytes as two integers:
>
> ```loft
> xs: vector<(integer, integer)> = [];  xs += [(1, 11)];  xs += [(2, 22)];
> s = c0(fn() -> integer { a = 0; for t in xs { a += t.0 * 1000 + t.1; } a });
> //  --interpret: 0, silently.  vector<(integer, P)>: len 0 then SIGSEGV.  --native: E0308.
> //  the same loop OUTSIDE the closure: 3033.
> ```
>
> A tuple of SCALARS fails too, and it carries no store — which is what rules an ownership
> explanation out and names the capture's SHAPE as the axis. The arm now also asks whether
> the slot holds a `DbRef` (`deps.contains(&u16::MAX)`, the spelling three neighbouring sites
> already read for the same question), which routes a capture to the auto-Reference store
> directly below it.
>
> Guard: `tests/scripts/1131-a-captured-collection-is-stored-as-a-handle-not-unpacked.loft`,
> which keeps the struct / nested-vector / keyed element types as controls — those are the
> @PLN93 shapes the tuple row fell outside of, and a fix that took one of them down would be
> worse than the defect.

> **D-clo-7 — value half CLOSED, leak half OPEN (2026-08-27, loft#1114).** `(L-CapHeap)`
> says a captured heap value is SHARED. A NULLABLE one was not: `closure_attr_type`
> recognised `Reference`, the keyed collections and `Vector`, and let `S?` fall through — so
> the capture kept its `__nullable<S>` enum type, was COPIED into the closure record INLINE
> while its dense twin was SHARED as a `DbRef`, and the body's read then applied the enum's
> payload offset on top of a record the write had placed without one. The lambda answered
> `4294967199`, with nothing saying so.
>
> `S?` IS a `DbRef` whose `rec == 0` means absent, which is why the cure is a peel and not a
> new storage class. `Data::nullable_struct_payload` answers the one-sided question in BOTH
> spellings — the `Optional(Reference(S))` the author writes and the `Enum(__nullable<S>,
> true)` the field rewrite produces — and that is the whole of it: **recognising only the
> spelling a site happens to see is what gives one value two layouts.** The same gap wore an
> ICE (the tail's type changes KIND between the passes, so the delivery arms differ and pass
> 2 grows an attribute pass 1 never minted) and a REFUSAL of a legal program (`Type::is_equal`
> had a peel for eight wrappers and none for `Optional`, so derived `==` compared the inner's
> deps and printed one type as two).
>
> ⚠ **The refusal was MASKING the wrong answer.** With the `Optional` peel applied and the
> capture still copied, a refused cell stops being refused and starts answering
> `4294967199` — a loud refusal traded for a silent wrong one. The peel is restricted to
> inners that CARRY DEPS, because a scalar has none and derived `==` then compares the SPEC,
> whose integer half is the layout-bearing WIDTH (loft#663): without that restriction `u8?`
> and a wider `integer?` become one type and `overflow(300)` answers `300`.
>
> ⚠ **And fixing the capture exposed a use-after-free behind it.** With the store shared, a
> caller's `??` over the fn-ref return LIFTED that join into a temp and freed it, releasing
> the captured record while the outer variable was still live — so a second lambda over the
> same variable read a released store. The licence was an empty return dep, and it is empty
> because `fnref_result_type` DROPS an index naming a hidden attribute on the stated grounds
> that *"the value arrives OWNED"*. `__closure` is a hidden attribute, and a captured value
> does not arrive owned: **a dep dropped as uninteresting is not a dep that was never there.**
> The lift now declines for a CAPTURING fn-ref and still fires for one that captures nothing.
>
> **The leak — first half CLOSED (2026-08-29, loft#1179), second half OPEN.** Both halves
> are the same sentence: *a direct call site mints the return buffer as a caller LOCAL it
> frees at scope exit, and the fn-ref path had no equivalent.*
>
> CLOSED — a lambda that BINDS its return to a local (`d = q ?? P{}; d`) leaked one store per
> call. `fn_call_ref` allocates one store per hidden return attribute because it cannot know
> which function the slot holds, and a callee that delivers its return some other way — it
> minted its own store, or the delivery slot was rebound to a borrow — left that store owned
> by nobody. `--native` never had it: its dispatch passes the null sentinel for a Reference
> return and frees an unfilled `__vc_hbuf` for a vector one, which is the same fact this side
> was missing. `State::fn_return` now releases every buffer the returning frame's call site
> allocated, keeping the one the callee handed back — identified by STORE, because a callee
> that delivered through the buffer may answer a record or a position inside it.
>
> That one free also closed loft#1180 (a lambda returning a captured struct's vector FIELD,
> both spellings) and made loft#1178's reservation safe to widen: reserving a return buffer
> for EVERY declared-collection lambda was already correct on `--native`, and the only thing
> wrong with it here was the unowned buffer.
>
> **OPEN: a lambda's `??`-default store discarded INLINE.** `g = fn(q: P?) -> P { q ?? P{} }`
> called as `g(null).n` leaks the default arm's store, one per call, on BOTH backends; the
> BOUND spelling is clean, and so is the named twin. The lambda's return dep names its
> parameter on the subject arm, so `returns_borrowed_view` calls the whole thing a borrow and
> `callref_owned_return` declines — the mint arm pays for the borrow arm's caution. It is a
> JOIN, and the direct-call branch beside it already knows what to do with one
> (`use_analysis::ownership_of`, lifting a `Join` only where the following bind is the runtime
> guard); the `CallRef` route does not ask.
>
> Guarded by `tests/scripts/1114-a-nullable-heap-capture-is-shared-like-its-dense-twin.loft`.

> **D-clo-6 — CLOSED (2026-08-27).** `(L-FnRef)` says a bare function name is a first-class
> value. It was not, for a function that carries TWO hidden `RefVar(Text)` work buffers:
> `g = nb; g()` crashed the interpreter. loft#1116, both halves closed the same day.
>
> A function acquires two the ordinary way — a text local AND a discharge accumulator, each
> promoted to a hidden `&text` out-param. That is legal for a NAMED function, whose own call
> sites lower against its known signature, and D-clo-4 records why forbidding it is not the
> cure (it moved five suite results). But the fn-ref ABI passes exactly ONE buffer, because
> a call site cannot know which function a fn-typed slot holds — so through a fn-ref the
> callee is entered short.
>
> **The `--native` half is closed.** Its dispatch arms are chosen by SIGNATURE, so a function
> nobody takes a reference to was reddening the build whenever some lambda shared its shape
> — the arm spent one buffer argument on both parameters (`E0499`). Extra buffers now get
> their own temporaries, which is sound on that backend and only there: native returns text
> OWNED and never threads the value back through the buffer, so the buffers type-check
> rather than deliver. Guarded by
> `tests/scripts/1116-a-fn-ref-arm-does-not-spend-one-buffer-twice.loft`.
>
> **The interpreter half is CLOSED too (2026-08-27, loft#1116).** There the buffer IS the
> delivery, so an extra temporary would have swallowed the result — and a `&text` is a
> pointer into the CALLER's frame, so the dispatcher cannot supply one that outlives its own
> return either. The count had to travel outward: the call site pushes what the WIDEST
> candidate of that signature could want (`Data::fnref_text_buffers`) and `fn_call_ref` pops
> what the actual target does not take, which is the same trim it already did for a target
> wanting none. One count, two readers.
>
> ⚠ **The other admissible cure on the issue — declining the fn-ref (`B-Ref-Reshape`'s
> precedent) — rested on a premise that had expired.** It was recorded as costing nothing
> *"since every such call faults today"*, and that was true when written; by the time it was
> taken up the `--native` half had landed and `g = nb_two; g()` ANSWERED there. Declining
> would have removed a working capability from one backend to make it match the other, and
> `(L-FnRef)` says the value is first-class in the first place. Re-measure a filed
> "nothing is lost" before building on it — a sibling fix can have made it false.
>
> Guarded by `tests/scripts/1116b-a-fn-ref-call-carries-every-text-buffer-its-target-wants.loft`,
> whose wide target holds DIFFERENT text in its two buffers on purpose: the obvious
> two-buffer function (`loc: text = "x"; return loc ?? "fb"`) has both buffers holding the
> same value, so reading the wrong one is invisible and that shape can only score a crash.

> **D-clo-5 — CLOSED (2026-08-27).** The third route to the same fault line, found by
> varying where `(L-Apply)` happens. `xs.map(fn(n: integer) -> text { return s; })` on a
> CAPTURING lambda panicked the interpreter with a corrupt `DbRef` while `--native`
> answered — a backend split, so neither backend alone could see it. loft#1115.
>
> Cause: the caller allocates the one hidden `RefVar(Text)` work buffer a text-returning
> fn-ref call hands its target, and `parse_operators` appends it for the ordinary `f(args)`
> spelling. `map` lowers its own `CallRef` and never appended it, so the callee was entered
> one DbRef span short and read its `__closure` from the wrong offset. The closure argument
> itself is NOT part of that injection — `fn_call_ref` reads it back from the 20-byte fn-ref
> slot — which is exactly why the same shape returning an integer, a struct or a boolean was
> always correct, and why the fault looked like a capture problem when it was a buffer one.
>
> Fixed in `parse_map` through one `callback_call_ref` helper. The buffer is drawn from
> `caller_text_buf`'s `__work_c<N>` sequence, not `work_text`'s `__work_<N>`: the map family
> early-returns on pass 1, so this mint is pass-2-only, and a pass-2-only mint on the shared
> counter shifts every later `__work_N` — loft#662's class. Guarded by
> `tests/scripts/1115-an-inline-callback-gets-the-text-buffer-its-abi-expects.loft`, whose
> native half is INERT by construction and says so.

> **D-clo-4 — CLOSED (2026-08-27).** `(L-Apply)` makes applying a closure a call, and
> [calls.md](calls.md) `(F-Return)` says `return e` exits the call with `e` — the same
> program as the tail spelling. A lambda whose body both `return`ed and discharged a null
> (`fn(n: integer) -> text { return s ?? "fallback"; }`) instead SIGSEGV'd the interpreter
> and failed to compile on `--native` (`E0499`), so the two spellings of one program
> disagreed and the rules settled which one was wrong. loft#1113.
>
> Cause: a text-returning lambda is handed **exactly one** hidden `RefVar(Text)` work
> buffer by the fn-ref call ABI — a call site holding a fn-typed slot cannot know which
> lambda is in it, so it injects one and the callee either uses it or has it popped
> (`State::fn_call_ref`). Two promotions can meet inside one body and neither consulted the
> other: `parse_return` promotes at the `return`, and the block tail promotes the `??` / `?`
> / `if` accumulator afterwards. The callee then carried TWO, the frame came up one DbRef
> span short, and it read its `__closure` slot from the wrong offset — loft#717's fault
> line, reached by a second route. Fixed where the buffers are minted (`text_return`, one
> `holds_text_work_buf` predicate now shared with the P227 placeholder it always had):
> the first promotion to ask takes the buffer, and a later text local stays a local,
> delivered by copy exactly as `SkipOwnedLocal` already prescribes.
>
> **The filed scope was a third of the defect.** It named three conditions — a closure, the
> `return` keyword, and a `??` yielding `text`. Only the closure is real: `?` reaches it
> (the other discharge rule), the null branch reaches it, and two plain text locals with no
> discharge anywhere reach it. What the shapes share is a SECOND buffer, not the spelling
> that asked for one. Guarded by
> `tests/scripts/1113-a-lambda-carries-one-text-work-buffer.loft` (falsified at `20e25e9a`:
> interpret exit 139 → 0, native exit 1 → 0).
>
> **Measured and rejected:** applying the one-buffer rule to NAMED functions too. It is the
> same ABI on paper — a named function whose signature matches a fn-ref's does reach the
> generated dispatch arm, which forwards one buffer twice and does not compile — but it
> moved five suite results, and `float=0.25` came back `0` through the sqlite bridges. A
> named function's ordinary call sites lower against a known signature and carry as many
> buffers as it declares. The named-function half is therefore still open, filed separately,
> and older than this fix.

> **D-clo-3 — CLOSED (2026-08-22).** `L-Escape` says a closure "may be stored in a
> variable or struct field", and said nothing about the slot being fresh — so **assigning**
> into a fn-typed struct field or vector element that already held one (`h.f = inc`,
> `v[0] = inc`) had to work, not merely fail better. It was refused on both backends, and
> refused by the wrong rule: the fn-ref read lowers to a `fn_ref_field_read` Block rather
> than the `Call`/`Var` place shapes the assignment dispatcher knows, so it was not
> recognised as writing ANYWHERE and fell through to *"Not implemented operation = for type
> function(…)"* — a message about the `=` operator, contradicted by the same field
> accepting the same value one line earlier.
>
> Fixed by peeling the read back to its place and handing each destination to the writer
> the LITERAL already uses — a struct field to `set_field`, a vector element to
> `fn_ref_slot_dnr` — so the two positions cannot come to different conclusions about what
> a fn-ref source may be. The P215/@P213 refusal for a non-inline source and the #247
> refusal for a capturing source in a collection are unchanged shipped decisions; what
> changed is that the assignment now REACHES them, which was loft#1072's "small half".
>
> Three things a slot that already holds a value needs that a fresh one does not, each
> measured failing on the way:
>
> * the closure half must be RELEASED. A non-capturing source left the previous closure
>   record in place, so the field read back as the new function paired with the old
>   closure, and `fn_call_ref` entered a callee that declares no closure with one pushed as
>   its hidden argument — a corrupt frame, not a stale value: the call returned misaligned
>   and the next read of an unrelated field faulted in `get_int`. A capturing source
>   orphaned the old record in the host's store instead — a leak that grows with the loop.
>   One `OpClearKeyed` against the `child_rec<…>` field closes both, through the same
>   `remove_claims` cascade that frees it when the host dies;
> * pass 1 must RECORD a capturing source, because the attribute's split layout comes from
>   `assigned_lambda_d_nr` being set in that pass and the read's byte offset is `u16::MAX`
>   there (the struct has no layout yet). The read site's own record of the attribute is
>   the only answer available on pass 1;
> * the host may be a `&` parameter — `RefVar(Reference)`, not `Reference`.
>
> Guarded by `tests/scripts/fn-ref-assigned-into-a-field.loft` (15 cells, both backends,
> value + a plain field read beside it + a 200-iteration loop for the leak), confirmed to
> fail on a pristine tree at 655ff4dd with 19 errors per backend. Fixes loft#1072.

> **D-clo-1 — CLOSED (2026-07-04).** The `|…|` short form now captures outer variables exactly
> like the `fn(){}` form — the two are pure syntactic sugar (L-Fn), the maker's intent.
> `parse_lambda_short` gained the closure-param setup block its sibling `parse_lambda` already
> had (add the `__closure` attribute + set `closure_param` so the body reads captures from the
> closure record), and builds its public `Function` type from the DECLARED params only (excluding
> the hidden `__closure`, so a `.map(f)` arity check still sees one param). Inert for a
> non-capturing lambda (no captures ⇒ no closure record ⇒ the block is a no-op). Guard
> `tests/scripts/85-short-lambda-capture.loft` (scalar + heap capture, both backends); 625 lib +
> native_scripts + interp suite green. (Residual, minor: a zero-arg `|| { … }` closure *assigned
> then called* has a separate parse edge — the `.map`/inline capturing forms all work.)

> **D-clo-2 — CLOSED (2026-07-04).** A stored short `|x|` lambda whose types could not be inferred
> (assigned without a type context, `g = |y| { y*2 }`) got a GARBAGE signature (a `text`/`void`
> default), and passing it to `.map` built a `vector<void>` result → a panic at `data.rs:4569`
> (`def(u32::MAX)`). The root cause was a crash where a **clean diagnostic** was already the intended
> outcome (the same lambda used standalone / called directly already errors "Cannot infer type for
> lambda parameter"). Fix: `parse_map` now guards a `void`/`Unknown` return (or `Unknown` param)
> from an un-inferrable fn-ref and emits the guiding "pass it inline / use `fn(x: T) -> R`"
> diagnostic instead of building the invalid result vector. The inline `.map(|y| …)` form (which
> has the element-type hint) and the long `fn(y: T) -> R` form are unaffected. Regression guard:
> `tests/leak.rs::dclo2_stored_short_lambda_map_no_crash` (parses without panicking, the guard
> diagnostic fires); 625 lib + interp + native_scripts green. (Making it *work* — inferring the
> stored lambda's types from the later `.map` source — is cross-statement inference, a separate
> enhancement; the crash → clean error is the fix.)

---

## Conformance

- **Both forms capture identically (`L-Fn`)** — `x=10; [1].map(|y| { y+x })[0]` is `11`, and the
  long form `[1].map(fn(y:integer)->integer{y+x})[0]` is also `11`; a captured heap value is shared
  (`b.v=8; [1].map(|z| { z+b.v })[0]` is `9`). A non-capturing `[1,2,3].map(|x| { x*2 })` is
  unchanged (`2`). (Guard `tests/scripts/85-short-lambda-capture.loft`.)
- **Capture semantics (`L-CapScalar` / `L-CapHeap`)** — a captured scalar reads its
  creation-time value; a captured struct reads its *current* field value (`b.v=9` ⇒ `9`).
- **First-class (`L-Escape`)** — a closure returned from a function, or stored in a struct field,
  works: `mk(7)()` is `7`; `h.f()` is `42`.
- **A fn-ref reaches every CONTAINER (`L-Escape`, measured 2026-08-22)** — vector element by
  literal and by `+= [f]`, keyed-collection value, struct-enum variant payload read
  per-variant, and struct-in-vector all carry one and call it back out, on both backends.
- **…and a place that ALREADY holds one takes a new fn-ref (`L-Escape`, D-clo-3)** — a live
  local, a live tuple member (guard
  `tests/scripts/fn-ref-reassignment-tops-up-the-pair.loft`), and a struct field, a vector
  element, an element's field, a field's element and a `&`-parameter's field (guard
  `tests/scripts/fn-ref-assigned-into-a-field.loft`), from a bare name, an inline lambda
  (capturing or not), a non-capturing local and a call — including over a field that already
  owns a closure record, and 200 times in a loop without the store growing. A source the
  LITERAL refuses (an `if`/`match` arm, P215; a capturing source into a collection, #247)
  is refused identically here, by the same diagnostic.
- **No-crash on an un-inferrable stored lambda (D-clo-2)** — `g = |y|{…}; xs.map(g)` now emits a
  clean "cannot infer" diagnostic on both backends, not a panic (guard
  `tests/leak.rs::dclo2_stored_short_lambda_map_no_crash`). The same diagnostic covers
  `any` / `all` / `sort_by` / `filter`: it fires at the LAMBDA, not per combinator.

Closures are a full first-class contract: construction, every container measured above, and
re-assignment into a place that already holds one. What a closure may not do is bounded by
two decisions rather than by gaps — one capture shape per fn-ref attribute, and no capturing
closure inside a collection (#247/@P213) or inside a struct that a collection holds (#318).
