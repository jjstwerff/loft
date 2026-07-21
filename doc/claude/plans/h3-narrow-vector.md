# H3 — narrow-width integers: three defects behind one symptom

> **Status: 2 of 3 FIXED.** Reported by the crawler consumer (`LOFT-HANDOFF` H3) as
> *"`vector<u32>` element write through a struct parameter is silently discarded;
> `vector<u16>`/`<i16>` reject index assignment"*. Both backends, both defects — this was
> never a backend divergence. The third (a `u32` value above `i32::MAX` reading back
> sign-extended) is a **structural gap, not a bug in a line**, and is still open.
> Probes: `h3-narrow-vector/probes/run.sh` — type × write-path × read-back, both backends.

## RESUME — start here

**One-line state:** the two defects the crawler could see are fixed at their root causes;
the one it could not see (it only ever used small `u32` values) needs a design decision
before it can be fixed — see § Open.

```bash
./doc/claude/plans/h3-narrow-vector/probes/run.sh    # expect 4 differing: c_u32_read + c_u32_field, both backends
```

## The reported scope was wrong in both directions

The handoff's matrix said the discriminator was *"the write happens inside a called fn,
via a struct param"*. It is not. Bisecting the reproducer one axis at a time:

| variant | result |
|---|---|
| loop-appended vector, value from a **parameter** | FAIL |
| literal vector, value from a **parameter** | FAIL |
| loop-appended vector, value a **constant** | pass |
| `i32` instead of `u32`, value from a parameter | pass |

So neither the struct, nor the parameter, nor the vector was load-bearing — the **value
being a runtime expression** was. Stripped all the way down, the entire defect is:

```loft
fn main() { n = 7; q = n as u32? ?? 0; print("{q}"); }   // prints 0, expected 7
```

No vector, no struct, no function. A constant operand hid it by const-folding before the
guard is emitted, which is why the crawler saw it only through a helper function.

## Defect 1 — `x as u32?` always yields null (FIXED)

`dn4_checked_cast` (`src/parser/operators.rs`) built the range guard with
`spec.max as i32`. `spec.max` is a `u32`, and `u32`'s maximum is 4_294_967_294, which
wraps to **-2**. The emitted guard was `0 <= v && v <= -2` — unsatisfiable, so every
checked cast returned null. `u8`/`u16`/`i8`/`i16`/`i32` were unaffected because theirs are
the only maxima that fit `i32`.

Fix: widen to `i64` and emit the bound with `int_literal`, which picks `Value::Long` when
the value does not fit `Value::Int`'s `i32` payload.

**This is what made it silent.** The cast returned null, the idiomatic `?? 0` supplied a
plausible zero, and a grid of zeroes reads as "not yet written" rather than "corrupt" —
crawler's collision field turned every wall passable and it surfaced only because a flood
test reported *every* heading leaking.

## Defect 2 — `vector<u16>`/`vector<i16>` reject index assignment (FIXED)

`v[i] = x` failed to compile with *"Cannot assign to attribute on type 'OpGetShortRaw'"*.
The read→write op mapping in `parse_assign` lists `OpGetShort` and `OpGetShortFull` but
not `OpGetShortRaw` — the narrow-**vector** 2-byte reader. All three share the same write
op (`OpSetShortRaw`, a `(val - min)` store); only their sentinel decode differs. A `u16`
struct FIELD worked throughout, because fields read through the other two.

Fix: fold `OpGetShortRaw` into the existing `OpGetShortFull` arm.

## Open — a `u32` above `i32::MAX` reads back sign-extended

```loft
v: vector<u32> = [4000000000];   // reads back -294967296
struct S { f: u32 }              // the same value in a struct field: identical failure
```

Both backends, every container — so this is the **width**, not the vector. The narrow-int
accessor family (`src/data.rs::NarrowIntKind`) is:

| width | ops | carries a `min` offset? |
|---|---|---|
| 1 | `OpGetByte` / `OpGetByteNullable` | yes |
| 2 | `OpGetShort` / `OpGetShortRaw` / `OpGetShortFull` | yes |
| **4** | **`OpGetInt4`** | **no** |

The `min` offset is exactly how `u8` and `u16` represent an unsigned range: they store
`val - min`. The 4-byte kind has no such parameter — `OpGetInt4` is a plain signed
`get_i32_raw` with `i32::MIN` as its null sentinel — so **there is no representation for
an unsigned 4-byte value at all**, and every `u32` above `i32::MAX` is unreachable.

That makes this a missing feature rather than a wrong line, and it needs a decision before
implementation, because `u32`'s declared range (`limit(0, 4_294_967_294)`) already reserves
its top code as a sentinel:

- add `Int4Raw`/`Int4Full` twins mirroring the 2-byte trio (a `min`-carrying
  `(val - min)` store) — consistent with the existing design, and the byte/short
  precedent says it works; costs a new op pair and a `takes_min()` widening; or
- store `u32` unsigned with `u32::MAX` as the sentinel and widen on load, which is
  cheaper but makes the 4-byte kind's encoding differ in kind from the 1/2-byte ones.

**Recommendation: the first.** It keeps one encoding rule across all narrow widths, and
the 2-byte trio is a working template to copy rather than a new design to validate.

Until it lands, `u32` is safe only below 2^31 — worth a diagnostic, since it is silent
today. Note the crawler's own workaround (use `i32`) is unaffected by this.

## Instrument

`probes/run.sh` — one file per (type, path) cell over
`u8 · i8 · u16 · i16 · u32 · i32 · integer · single`, crossed with **direct** write,
**via-fn** write through a struct parameter, and a pure **read** round-trip. Cells assert
against a hand-computed expectation printed in the same line, never against a second
backend, since a width bug lowers the same wrong way on both.

Two harness lessons from building it: a cell that fails to COMPILE must count as a
failure, not a blank (the u16/i16 cells produce no stdout at all), and a fixture error
reads exactly like a defect — the `single` cells failed first time on a missing
`as single` cast of mine, not on anything in the compiler.
