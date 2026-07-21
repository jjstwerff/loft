# H3 — narrow-width integers: three defects behind one symptom

> **Status: ALL THREE FIXED.** Reported by the crawler consumer (`LOFT-HANDOFF` H3) as
> *"`vector<u32>` element write through a struct parameter is silently discarded;
> `vector<u16>`/`<i16>` reject index assignment"*. Every defect reproduced on both
> backends — this was never a backend divergence. Probes:
> `h3-narrow-vector/probes/run.sh` (type × write-path × read-back) and
> `h3-narrow-vector/bytecode-comparisons/width-corpus.loft` (one function per 4-byte
> storage path beside its working 2-byte analogue).

## RESUME — start here

**One-line state:** done. Two defects were wrong lines; the third was a missing
representation — loft had no unsigned 4-byte accessor at all — now supplied by the
`OpGetInt4Raw` / `OpGetInt4Full` / `OpSetInt4Raw` trio.

```bash
P=./doc/claude/plans/h3-narrow-vector
$P/probes/run.sh                                    # expect 0 differing
loft --interpret $P/bytecode-comparisons/width-corpus.loft   # every line must echo its `|` half
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

## Defect 3 — there was no unsigned 4-byte representation at all (FIXED)

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

That makes it a missing feature rather than a wrong line: no encoding existed for the
values, so nothing could be "corrected" in place.

**The fix — `OpGetInt4Raw` / `OpGetInt4Full` / `OpSetInt4Raw`,** the 4-byte twins of the
2-byte trio, selected by one new fact on the type (`IntegerSpec::unsigned_wide()` — the
declared range is non-negative AND runs past `i32::MAX`):

| kind | reserves a sentinel? | used for |
|---|---|---|
| `Int4Raw` | yes, `u32::MAX` | a nullable slot, or a narrow-vector element |
| `Int4Full` | no — full 2³² | a non-null field |
| `Int4` (unchanged) | yes, `i32::MIN` | everything else 4-byte, `i32` above all |

Two decisions worth keeping:

- **Storage is a plain native `u32`, not the 2-byte trio's `(val - min)` bias.** The bias
  is what lets 1- and 2-byte slots express an unsigned range, but at 4 bytes the value
  already fills the slot, so a bias buys nothing and would change the stored bytes. Native
  layout is what a binary format expects, so this stays a *decode* change.
- **`i32` deliberately does NOT move.** Mirroring the trio literally would have given it
  `min = -2147483647` and a biased encoding, rewriting the bytes of every `i32` field —
  read by raw copies, FFI and codegen outside the narrow-int family. `unsigned_wide()`'s
  `min >= 0` half is what holds that line (it also excludes the WIDE 8-byte template,
  which sets `max == u32::MAX` purely as a "wider than i32" marker while keeping a
  negative `min`).

It also closes a defect nobody had reported: **2147483648 is a legal `u32` and is exactly
the `i32::MIN` sentinel bit pattern**, so that one value used to read back as *null*. The
unsigned sentinel is `u32::MAX`, which `u32`'s declared range (`limit(0, 4294967294)`)
already excludes — so no legal value collides with null, at any width. That matches what
LOFT.md § nullable narrow fields already promised: *"`u32` covers 0..=4_294_967_294 — one
32-bit code reserved."*

**Proof it changed only what it meant to.** `bytecode-comparisons/width-corpus.loft` holds
one function per storage path; comparing `loft introspect` before and after (normalising
the def_nr/address shift that adding three stdlib ops causes), the only functions whose
emitted ops move are the `u32` ones. Every `i32` and `u16` path — IR *and* generated Rust
— is unchanged.

**A note on the chokepoint.** Adding the readers immediately reproduced defect 2 for the
new ops: `v[i] = x` stopped compiling with *"Cannot assign to attribute on type
'OpGetInt4Raw'"* until they were added to the same read→write map. That map is a real
chokepoint and an easy one to miss — the probe matrix caught it in one run.

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
