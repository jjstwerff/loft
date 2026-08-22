<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN146 F7a — will `shapes` accept a derived proxy?

The phase gate for **F7a**, and the shape decision it hands to **F7**. The plan and its
other phases are in [README.md](README.md).

## The question as filed, and the answer

> Red if the shape kinds do not meet — `shapes` ships `Rect`/`Circle` and a derived hull
> is neither.

They do not meet, and the phase is right that they don't: `shapes` has no polygon kind and
no way to express one. But that reads as *"F7 needs a new shape kind"*, and the measurement
says the opposite. **Do not derive a hull.** A proxy built as a *set* of `shapes::Rect`
is both expressible today and **tighter than the convex hull on every statistic**.

## What the corpus says

[`probes/f7a_alpha.py`](probes/f7a_alpha.py) derives a proxy from each of the 36 alpha-
carrying goldens ([W0](W0.md)'s corpus) and measures the overshoot F7's own contract asks
for — `proxy ⊇ opaque`, `overshoot = (proxy_area − opaque_area) / opaque_area`:

| proxy | mean | median | worst | ≤ +100 % | expressible in `shapes` today |
|---|---:|---:|---:|---:|---|
| one `Rect` (tight AABB) | +150.6 % | +104.8 % | +761.3 % | 17/36 | ✅ |
| one `Circle` | +411.8 % | +255.8 % | +1671.3 % | 7/36 | ✅ |
| convex hull | +54.9 % | +42.4 % | +259.9 % | 30/36 | ❌ no kind |
| 8 `Rect` bands | +52.2 % | +44.6 % | +274.1 % | 33/36 | ✅ |
| **16 `Rect` bands** | **+36.7 %** | **+29.2 %** | **+223.0 %** | **35/36** | ✅ |
| 32 `Rect` bands | +26.4 % | +16.5 % | +199.6 % | 35/36 | ✅ |

A band decomposition splits the opaque mask into *k* strips along one axis and takes each
strip's tight box. It is one pass over the alpha the packer already reads, every box
contains its strip's texels by construction, and at k=16 it beats the hull on mean, median,
worst case and the count of sprites inside a doubling. So the shape F7 should derive is a
**`vector<Rect>`**, and the reason is measurement rather than convenience.

Two smaller results the packer wants:

- **The better axis is per sprite, not a constant.** 17 of 36 sprites are tighter banded by
  column than by row. Choosing per sprite takes the median from +29.2 % to +22.8 % for the
  cost of running the derivation twice and keeping the smaller.
- **Per-component boxes are a trap.** Boxing each connected component instead of banding is
  *worse* than a single box at the mean (+169.7 % against +150.6 %) — `ammo` goes to
  +1630 %, because its three arrows are diagonal and a diagonal component's own AABB is
  nearly empty. Band first; components never help.

## The one sprite that stays bad

`ammo` is +223 % at 16 bands and is the worst case at every k, on either axis. Three thin
diagonal arrows are the adversarial case for any axis-aligned decomposition, and no band
count fixes it — +199.6 % at k=32. That is the honest shape of F7's bound: **`≤ +100 %`
covers 35 of 36 sprites, and the packer should report the one it cannot meet** rather than
silently shipping a proxy twice the size of its art. A bound nothing can violate is not a
bound.

## The probe

[`probes/f7a_shapes.loft`](probes/f7a_shapes.loft) builds `wand`'s real 8-band proxy —
`wand` is the corpus's worst single-box case, a diagonal stick whose AABB is +631 % — and
runs it through `shapes`' own predicate:

- `vector<shapes::Rect>` as a struct field **compiles and composes across the package
  boundary**, on `--interpret` and `--native` alike. That was the structural risk and it
  is not one.
- The AABB and the derived proxy **disagree where they must**: a mover in the wand's empty
  top-left corner is a hit under the single box and a miss under the eight, while a mover
  on the stick is a hit under both. Inverting that assertion turns the probe red, so it is
  a test rather than a decoration.
- `proxy_hits` — does any box hit this one — is nine lines and every consumer will write
  it. `shapes` should grow it, together with the `Proxy` type, when F7 lands.

## What this hands F7

1. Derive **k=16 bands on the tighter axis**, not a hull, not a component set.
2. Emit `vector<Rect>` into the pack; `shapes` consumes it unchanged.
3. Gate at **overshoot ≤ +100 %** per sprite, and *report* the sprites that miss it.
4. Add `Proxy` + `proxy_hits` to `shapes` so the set test has one home instead of one per
   consumer. This is `shapes`' first real consumer, which is what F7a was cut to establish.
