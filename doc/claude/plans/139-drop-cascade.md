<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN139 — a droppable's owner drops it

**Canonical:** [loft-lang/plans#139](https://github.com/loft-lang/plans/issues/139) ·
**Bug:** [loft#849](https://github.com/loft-lang/loft/issues/849) ·
**Scope decision:** [DESIGN_DECISIONS.md § C111](../DESIGN_DECISIONS.md)

## The rule

A droppable copied into a container is a **MOVE**: the container's copy becomes the owner, the
source no longer drops, and the container's death drops what it owns. Decided over the refusal
alternative because the sanctioned workaround (`disown` after constructing the container)
requires the droppable to BE a field — a declaration-site refusal rejects its own cure, and with
collections in scope it would have to cover `vector<T>` too.

Scope boundary decided with it and recorded as **C111**: the cascade reaches a container's
DEATH, not an element's REMOVAL. `v.remove(i)` / `v[i] = x` do not drop the displaced element.

> A drop runs when the value's OWNER dies. Taking a value out of its owner does not.

## Where it stands

Two halves of loft#849 are already fixed on `tuxedo-decisions`:

- `fc3fb2c3` — a consumed MOVE source no longer drops. `OpCopyRecord(src, dst, kt|0x8000)` frees
  the source store, and the lift temp kept naming it, so a vector literal of two droppables
  closed the second's resource twice and the first's never (`LOFT_STRICT_STORES`: USE AFTER FREE,
  4 violations → 0).
- The narrowing on the issue: the source's early drop is only VISIBLE when the container outlives
  the scope, which is why a same-scope test reads as working.

What remains is the cascade itself, plus the field-side half of the move.

## The architectural finding — why this is a plan and not a patch

**The drop is emitted where the layout is not available.** `scopes::check(data: &mut Data)` is
where free/drop sites are computed, and it has `Data` only. Field offsets live in the schema
(`Stores`, via `database.position(kt, name)`), which `Data` does not carry and `Attribute` does
not store. So `OpGetField` — the one node a field cascade needs — cannot be built there at all.

Three routes were evaluated against that:

1. **Pass `&Stores` into `scopes::check`.** Mechanical (~8 call sites) and it unblocks struct
   fields and enum-variant dispatch. It does NOT unblock vector elements: those need a loop
   (`_vector_N` alias + index var + break conditions), which means minting variables and a loop
   scope *after* the scan, where slot allocation and the loop-scope bookkeeping already ran.
2. **Synthesize a per-type cascade FUNCTION from generated loft source**, so fields, enum match
   and the element loop all fall out of normal parsing. Blocked: `parse_str` resets `Data` and
   re-parses the whole text, so there is no snippet-injection path — it would need a third full
   parse, against a two-pass contract that is already load-bearing (the H5 guard).
3. **Synthesize the cascade function programmatically**, the `synth_nullable_par_wrapper`
   precedent (build the def, the var table, and a `Value` body). Field access comes free via
   `get_val`'s offset coercion, and enum dispatch is an `If` on the discriminator. The element
   loop is hand-built IR either way — this is the only route that reaches all three, and it is
   the recommended one.

So the vector half needs a designed mechanism, not an extra argument, and that is the piece that
makes this plan-sized rather than a session-tail change.

## Staging

Each stage lands against the matrix below, on BOTH backends, with the full gate green.

- **A — the query. SHIPPED.** `Data::owns_droppable(T)`: does T have a hook, or transitively
  contain a member type that does. Cycle-guarded, and deliberately a different fact from
  `drop_hook_nr` — a wrapper with no hook of its own around a type that has one answers `false`
  there and `true` here, which is exactly loft#849. Landed INERT: 78 pure additions to `data.rs`
  with zero callers, so it cannot change emitted output by construction (the stronger claim than
  a byte-identical diff, which only samples). Tests in `tests/owns_droppable.rs`: every `true`
  cell has a `false` twin differing in ONE axis, so a query that answered `true` for every record
  type fails half of them. Two cycle cases — a self-referential node, and a two-type cycle where
  the hook is reachable only through the back edge, asserted from BOTH ends (a guard that cached
  its `false` for a revisited def would answer differently depending on which end asked).
- **B — struct fields. SHIPPED.** `Parser::synth_drop_cascades` gives every type that owns a
  droppable FIELD a synthesized `t_<LEN><Type>_OpDropAll`: own hook first, then fields in
  reverse declaration order, each calling the field type's own cascade so nesting needs no
  special case. Declared in two phases (all names, then all bodies) so a nested chain does not
  depend on definition order. Runs at the five pass-2 tails beside `check_reshape_under_reference`
  — where every type and every hook are known — and is idempotent. `Data::drop_cascade_nr` is
  what a drop site calls: the cascade when one exists, the bare hook otherwise, so a program
  with no containers is unchanged.
- **C — the field-side move. SHIPPED.** A copy that hands its source off no longer leaves the
  source dropping: the `0x8000` move into an element (already in `fc3fb2c3`) and now a copy whose
  DESTINATION is a container field. **Scoped to containers that actually have a cascade**
  (`Data::has_drop_cascade`) — a copy into an enum payload or a collection element is not yet
  cascaded, so it is not treated as a transfer and its source keeps dropping. That scoping is
  what keeps the staging honest; without it stages D/E's shapes turn today's early release into
  a silent leak. The source also peels through a BLOCK tail, since an `Object` construction
  reaches the copy as the block that builds it (`Nest { s: S { … } }` double-released without it).
  Cells c1/c2/c4/c5/c6/c9 now match; guard: `tests/scripts/139-drop-cascade-fields.loft`.
- **D — enum payloads.** Variant dispatch on the discriminator; only variants with a droppable
  payload get an arm. Cell c3. **This is @PLN138's shape** — the registry wraps its cursor in an
  enum — so it is the stage the consumer is actually waiting on.
- **E — collection elements.** The hand-built loop. Cells c7/c8. **Both LEAK until this
  lands** — c7 since `fc3fb2c3` (the double-close fix), c8 since stage C moved its `S` temp's
  ownership into the element. Leak, not corruption, and `LOFT_STRICT_STORES` is clean on the
  matrix; but it is the gap that makes the cascade partial and it should not sit open long.
- **F — the contract.** INTERFACES.md § `OpDrop` rewritten to the owner rule, CAVEATS entry
  retired, C111 cross-linked.

## Known hazard, not yet designed

**A droppable moved into TWO containers** (`s1 = S{h:c}; s2 = S{h:c}`) becomes a double-close
once the cascade exists — today it is merely a leak. Rust prevents this with move checking, which
loft does not have. It is statically detectable in the common case (a var that is the source of
more than one container-field copy), so the candidate is a `warning`-tier diagnostic; that gates,
per the tier rule, because ignoring it produces a wrong result. Decide in stage C.

## The matrix

Hand-computed from the rule, captured against today's build so every cell has a before. Failing
today: c4 (drops before the container is alive), c6 (inner before outer), c7 (no drop at all),
c9 (the reported early close). Passing cells are anchors — c1/c2/c3/c5/c8/c13 pass today only
because the SOURCE happens to die at the same scope end, so they must keep passing for the NEW
reason.

| cell | shape | expected |
|---|---|---|
| c1 | struct field, source local | one drop, after `(alive)` |
| c2 | struct field, inline temp | one drop, after `(alive)` |
| c3 | enum payload | one drop, after `(alive)` |
| c4 | nested struct-in-struct | one drop, after `(alive)` |
| c5 | two droppable fields | reverse-declaration: 6 then 5 |
| c6 | container with its OWN hook | outer:7 then 70 |
| c7 | `vector<H>` two elements | 8 then 9 |
| c8 | `vector<S>` of containers | 10 |
| c9 | container returned, dies in caller | one drop, after `(alive)` |
| c10 | plain local, no container — CONTROL | unchanged |
| c11 | no droppable anywhere — CONTROL | nothing |
| c12 | enum unit variant, no payload — CONTROL | nothing |
| c13 | container in a loop | per iteration |
| c14 | two droppables, never containered — CONTROL | 51 then 50 |

Probe: the matrix source is at the bottom of this file.
Regression guards already in tree: `tests/scripts/849-move-copy-source-drop.loft`.

## The matrix probe

Kept here rather than in `tests/scripts/` because four cells FAIL today — it becomes a
regression guard when stage E lands. Run it on both backends and read it against the table
above; a drop printed before `(alive)` is premature, a missing one is a leak, a repeat is a
double-close.

```loft
// Expectation matrix for the STATIC DROP CASCADE. Each case prints an (alive)
// marker; a drop before it is premature, a missing drop is a leak, a repeat is a
// double-close. Expectations are hand-computed from the rule:
//   one drop per resource, at the death of whatever finally owns it;
//   a container's OWN hook runs before the fields it owns.
struct H { id: integer }
fn OpDrop(self: H) { if self.id != 0 { println("    DROP:{self.id}") } }
fn mk(id: integer) -> H { return H { id: id }; }

struct S { h: H }
struct Nest { s: S }
struct Two { a: H, b: H }
struct WithHook { h: H, tag: integer }
fn OpDrop(self: WithHook) { println("    DROP-outer:{self.tag}") }
enum W { WH { h: H }, WNone }
struct Plain { n: integer }          // no droppable anywhere — must stay untouched

fn c1()  { println("  c1 struct field, source local            exp: (alive) 1");
           c = mk(1); s = S { h: c }; println("    (alive {s.h.id})"); }
fn c2()  { println("  c2 struct field, inline temp             exp: (alive) 2");
           s = S { h: mk(2) }; println("    (alive {s.h.id})"); }
fn c3()  { println("  c3 enum payload                          exp: (alive) 3");
           c = mk(3); w: W = WH { h: c }; println("    (alive)"); }
fn c4()  { println("  c4 nested struct-in-struct               exp: (alive) 4");
           n = Nest { s: S { h: mk(4) } }; println("    (alive {n.s.h.id})"); }
fn c5()  { println("  c5 two droppable fields                  exp: (alive) 6 5");
           t = Two { a: mk(5), b: mk(6) }; println("    (alive {t.a.id}{t.b.id})"); }
fn c6()  { println("  c6 container with its OWN hook           exp: (alive) outer:7 70");
           x = WithHook { h: mk(70), tag: 7 }; println("    (alive {x.h.id})"); }
fn c7()  { println("  c7 vector of droppables                  exp: (alive) 8 9");
           v: vector<H> = [mk(8), mk(9)]; println("    (alive {len(v)})"); }
fn c8()  { println("  c8 vector of containers                  exp: (alive) 10");
           v: vector<S> = [S { h: mk(10) }]; println("    (alive {len(v)})"); }
fn c9m() -> S { return S { h: mk(11) }; }
fn c9()  { println("  c9 container RETURNED, dies in caller    exp: (alive) 11");
           s = c9m(); println("    (alive {s.h.id})"); }
fn c10() { println("  c10 CONTROL plain local, no container    exp: (alive) 12");
           c = mk(12); println("    (alive {c.id})"); }
fn c11() { println("  c11 CONTROL no droppable anywhere        exp: (alive) nothing");
           p = Plain { n: 1 }; println("    (alive {p.n})"); }
fn c12() { println("  c12 enum unit variant, no payload        exp: (alive) nothing");
           w: W = WNone; println("    (alive)"); }
fn c13() { println("  c13 container in a loop, per iteration   exp: 41 (alive) 42 (alive)");
           for i in 41..43 { s = S { h: mk(i) }; println("    (alive {s.h.id})"); } }
fn c14() { println("  c14 CONTROL droppable never containered  exp: (alive) 51 50");
           a = mk(50); b = mk(51); println("    (alive {a.id}{b.id})"); }

fn main() {
  c1(); c2(); c3(); c4(); c5(); c6(); c7(); c8(); c9(); c10(); c11(); c12(); c13(); c14();
  println("end");
}
```
