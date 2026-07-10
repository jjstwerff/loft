<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN85 — generic-monomorph text-return promotion (category A, 8 leakers)

Arc opened 2026-07-10 after the n3 / p54_b6 / p227 promotion fixes (16→9 leakers).
Closes `plan17_b`, `plan17_printable`, `p243`, `p329`×3, `p330`×2 — all the
generic-monomorph text/tuple returns.

## Root cause (bytecode-pinned)

A generic and its non-generic twin differ ONLY in promotion:

```
fn to_text2b(x: integer)      -> text { x.to_text() }   // fn n_to_text2b(x, ___tret_1:&text) -> text  ✅ leak-0
fn to_text2<T: Printable>(x:T)-> text { x.to_text() }   // fn t_7integer_to_text2(x) -> String        ✗ leak-1
```

The non-generic goes through `parse_block`'s `do_tret_bind` + `text_return`
(`control.rs`), which adds the hidden `___tret_1:&text` buffer and rewrites the
tail to `AppendStackText(___tret_1, __work); return ___tret_1`. The monomorph is
built by **IR substitution** (`try_generic_instantiation`, `parser/mod.rs:3045`),
NOT by parsing — so the parse-time promotion is **never engaged**. The monomorph
returns an owned `String` by value; native RAII drops it (clean) but the
interpreter orphans the buffer → leak. This is the SAME unpromoted-owned-text
class as n3/p54_b6/p227, one level down (inside instantiation).

Confirmed the promotion path is bypassed for generics (no `do_tret_bind`/
`text_return` runs for the template OR the monomorph).

## Routes considered

- **C — promote the TEMPLATE** (so every monomorph inherits the buffer via
  substitution): **INFEASIBLE.** The template tail `x.to_text()` has `x:T`; its
  delivery shape (NativeCall vs UserCall vs view) is unknown until `T` is
  concrete, so `classify_text_return` cannot decide promotion at template parse.
- **A — re-parse the monomorph body** through `parse_block` with `T` bound to the
  concrete type: cleanest (reuses ALL machinery incl. `do_tret_bind`), but needs
  the parser to re-enter the template body from source with a type binding — a big
  change to instantiation (currently pure IR substitution, no re-parse).
- **B — re-run the promotion on the MINTED monomorph** (RECOMMENDED, contained):
  at the hook point (`parser/mod.rs:3178`, after `new_code`/`vars` are set on the
  monomorph def), CONTEXT-SWAP: save `self.context`/`self.vars`, point them at the
  monomorph, apply the `do_tret_bind` rebind (`Set(__tret, tail); __tret`) to the
  monomorph body, call `text_return(&[__tret])` to stamp the `&text` buffer, set
  the returned type to `Text(frame1(__tret))`, then restore. `text_return`
  (`control.rs:4798`) is parse-coupled (mutates `self.context`'s attrs + `self.vars`
  in place), so the swap is required to reuse it without re-implementing.

## Route-B implementation spec

1. Gate: only when the monomorph's `returned` is `Text`/`text?` (or a `Tuple` with
   ≥1 owned text element for p329/p330) AND the tail classifies promotable.
2. The tail `x.to_text()` substitutes to a call to the concrete `t_7integer_to_text`
   (itself a promoted native buffer fn) — verify it classifies `NativeCall`
   (promotes UNGATED, no backward-ref needed) vs `UserCall` (needs the callee minted
   before the monomorph — the def_nr gate would then block it, the known forward-ref
   wall). The discarded/bound cases (`z = to_text2(7)`) need ONLY the monomorph
   promotion; the RETURNED cases (`run()->text{ first(nums) }`) ALSO need `run` to
   promote its own return, which the def_nr gate blocks (monomorph minted after
   `run`) — so route B alone may close the bound cases but not the returned ones.
3. Tuple returns (p329/p330): `text_return`'s `SkipTupleLocal`/`TupleElement` path +
   the `__ret_text_N` per-element hoist — the monomorph must re-run that too.

## Gates (MANDATORY — delicate generic path, slice-1 has 3 reverted attempts)

- Working bytecode: the monomorph must emit BYTE-IDENTICAL to the non-generic twin
  (`to_text2b` above is the proven-clean target). Prove on BOTH backends.
- Pass-stability: `predict_generic_return_type` (`parser/mod.rs:3002`) predicts the
  pass-1 return type; if route B changes the monomorph's returned type (adds the
  buffer), the prediction must AGREE across passes or a receiving var "changes type"
  (#395 class). Re-run the both-pass H5 checks.
- Full suite + the p329/p330/plan17 tests + `framework/verify.sh`.

## Status: route B IMPLEMENTED for `-> text` monomorphs (2026-07-10)

`Parser::promote_monomorph_text_return` (`control.rs`) — the context-swap
(`self.context` + `self.vars` onto the monomorph, replicate `do_tret_bind`,
call `text_return`), wired at `parser/mod.rs`'s `try_generic_instantiation` before
it returns `d_nr`. The substituted tail `x.to_text()` → concrete
`t_7integer_to_text` classifies `NativeCall` → promotes UNGATED, so the monomorph
gains `var____tret_1: &mut String` and delivers into it — byte-shape matches the
non-generic twin.

**Closed (3):** `plan17_b`, `plan17_printable`, `p243` — the generic `.to_text()`
BOUND cases (`test_value = { first(nums) }`, harness shape). Verified leak-0 +
correct on both backends; full suite 2738-green.

**Still open (5), two distinct residues:**
- **Tuple returns — `p329`×3, `p330`×2 — DIFFERENT CLASS (not route B).**
  Diagnosed 2026-07-10. A `-> (text, text)`/`-> (text, integer)` generic monomorph
  returns the tuple BY VALUE; the scope-pass per-element hoist DID run (the body has
  `var___ret_text_1 = x.to_text().to_string(); return (var___ret_text_1.to_string(),
  …)`), but `__ret_text_1` is `skip_free` (to outlive the frame) and is ORPHANED on
  interp (native RAII drops it). Correct matrix (element access `.0`/`.1`): the leak
  is present for BOTH the text element AND the integer element read — so it is
  independent of consumption; the text element's owned temp simply never frees. This
  is the SAME `skip_free`-orphan class as the `??`/ncc leak, NOT the
  unpromoted-owned-return class route B fixes — `promote_monomorph_text_return`
  cannot touch it.
  - The CLEAN path is the non-generic ABI: `-> (text, text)` is rewritten to the
    `__tuple<text,text>` synthetic STRUCT (`needs_tuple_rewrite`, definitions.rs), a
    heap Reference freed normally. The generic deliberately keeps the BARE-VALUE
    tuple ABI (`tuple_return_rewrite`'s `from_tv` gate, `parser/mod.rs:2967`), whose
    doc-comment warns "rewriting the literal tuple breaks the value-tuple generic
    returns (p329/p330/p240/plan17)" — i.e. someone tried routing generics through
    `__tuple` and it regressed. So the two viable fixes are both delicate: (a) free
    `__ret_text_N` at the interp tuple-return without UAF (the shared skip_free-orphan
    problem), or (b) re-open the `__tuple`-for-generics route past the documented
    landmine. A separate arc, sibling to ncc — do NOT fold onto route B.
- **Returned (not bound) monomorph text — e.g. `run() -> text { first(nums) }`.**
  The monomorph promotes, but `run` returning it does NOT (the def_nr gate blocks
  `run`'s own promotion — the monomorph is minted AFTER `run`, reads forward). This
  is the forward-ref pre-pass half, unchanged by route B. (The harness tests are
  the BOUND shape, so this doesn't block the 3 above — but a real `-> text` wrapper
  around a generic call still leaks.)
