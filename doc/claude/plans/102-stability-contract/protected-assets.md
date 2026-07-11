<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Protected assets — the proprietary-format mode, and why the freeze reserves for it

> **Status: design rationale, 2026-07-10.** Emerged from the @PLN102 layout freeze audit
> ([formal-audit.md](formal-audit.md) § Ownership/heap/layout). Not a committed feature — this
> records *why* the layout freeze draws its line where it does (freeze the logical layer, leave the
> physical byte-encoding permutable), so a future protected-asset mode stays shippable after 1.0.
> **The legal section is general information, NOT legal advice** — it is jurisdictional and a real
> product needs a lawyer.

## Why this matters — for the ecosystem, not the maker

loft's own games ship **free, with free unique assets**, so the maker needs none of this. But a
platform is judged by whom it *can't* serve: a **small indie developer** who sells or licenses
paid assets (animations, effects, models) has no such luxury — they need to ship those assets in a
game without handing every buyer a free, reusable copy. If loft cannot support that, it is
unusable for a large, commercial slice of exactly the people [GOALS.md](../../GOALS.md) /
[BROADENING.md](../../BROADENING.md) want it to reach. So the protected-asset mode is a
**platform-viability** feature for the ecosystem, and the *freeze* must not foreclose it — hence
this note.

## The threat model and the realistic bar

From the Unity comparison (the anchor for this whole thread): **anything the client must hold to
use, the client can extract.** Client-side crypto/obfuscation only raises cost; the only *robust*
protection is server-side (never ship the sensitive thing). For *shippable* game assets that must
run on the player's GPU, server-side isn't an option, so the realistic bar is: make a ripped asset
file **useless without that specific build**, and — the load-bearing part — make breaking it
**legally actionable**. The technical measure exists to establish the *legal* position, not to be
uncrackable.

### The success criterion (the bar — and the bound on effort)

> **The target is: reuse is *troublesome to a skilled adversary* AND *legally actionable*.
> Explicitly NOT: uncrackable.**

Perfect protection is impossible client-side, so the useful question is never "can it be broken"
but "**is it more trouble than it's worth**." A good yardstick is a *traditional hacker* — a
demanding adversary who knows the real cost of reverse-engineering: if the measure troubles *them*,
it is far above the casual-modder threshold, and casual/generic reuse is where essentially all the
threat volume lives. The two layers then cover the whole distribution: **troublesome** deters the
many (who would rip a two-click extract), and the **legal TPM layer** covers the capable few (for
whom the question becomes "is it worth the anti-circumvention exposure" — usually no).

This bar is also a **bound on effort**: because "uncrackable" is neither achievable nor required,
the measure must NOT be gold-plated — no heavy crypto, no exotic runtime, no hot-path cost. The
per-export permutation behind the getter axiom is troublesome in the *structural* ways that matter
(no cross-build tool; the mapping scattered through compiled code / shader / VRAM; per-build
reverse-engineering required) at essentially zero runtime cost — which is the right amount. Future
work should hit this bar and stop, not chase impossibility.

## The design (the technical measure)

- **Two layout modes of the same logical types.**
  - **Canonical / durable** — self-describing, portable, stable, hash-verified. For the @PLN43
    durable store, save files, tooling. This is the frozen contract. **Not a protection measure —
    and must not pretend to be** (a self-describing format hands over its own schema).
  - **Protected** — **per-export permuted** physical layout (field offsets + enum discriminant
    *values*), schema **stripped**, the un-permute/decode **compiled into that build** (native
    code, or a shader). Opaque without the build; each export means something different, so no
    generic ripper tool decodes across games.
- **The getter axiom is the seam.** Field/enum access is a getter axiom whose *logical* contract
  (read field X → its value) is frozen, and whose *physical* realization (offset, permuted
  discriminant, decode) is behind it and unfrozen. It is the one place a build specializes to its
  permutation, so no per-access special codegen is needed. loft is already most of the way there:
  `s.field` → `OpGetField(value, offset)` is getter-shaped, and the offset is the permutable fact.
- **mmap-friendly + GPU-friendly.** Because the asset is mmap'd (on-disk == in-memory, no
  load-time decode), the permutation *is* the physical layout read in place. And because the
  animation hot path lives on the **GPU**, the CPU never loops per-element — it marshals to GPU
  buffers at upload, so a decoding getter is amortized. Stronger still: push the un-permute **into
  the shader**, so the permuted asset goes mmap → GPU buffer zero-copy and stays permuted in VRAM;
  the shader un-permutes per-vertex (cheap on the GPU that is already skinning). Decode location
  (CPU-at-upload vs GPU-in-shader) is an unfrozen implementation choice below the getter axiom.
- **Enum ordering stays logical.** Ordering/comparison is on the declaration index, never the
  stored discriminant value — otherwise permuting the discriminant would silently change every
  enum compare/sort per build. Safe *because* protected assets are library-only data never ordered
  as keys.

## The legal framing (why the proprietary format is load-bearing)

A proprietary/opaque asset format is not just a deterrent; in most jurisdictions it is the
**prerequisite for the strongest protection**. Three layers:

1. **Copyright** — protects the assets by default, format-independent, but harder to enforce (must
   prove copying, substantiality, damages; face fair-use/mod defenses).
2. **Contract (the license/EULA)** — a "no reverse-engineering / no extraction" term; a proprietary
   format gives it something concrete to bind.
3. **Anti-circumvention** — **the layer the technical measure unlocks.** DMCA §1201 (US), EU
   InfoSoc Directive Art. 6, and equivalents make it independently unlawful to **circumvent a
   technological protection measure (TPM)** controlling access to a copyrighted work. If the asset
   ships behind a genuine TPM, *breaking the format is itself the violation* — separate from, and
   usually easier to enforce than, proving infringement (the act of circumvention is the wrong).

So the proprietary-format work is what upgrades an asset seller from **copyright only** to
**copyright + license + anti-circumvention** — the same three-layer position a Unity/Unreal asset
gets, rather than copyright alone.

**It only counts if it qualifies as a TPM**, and that maps exactly onto the two-modes split: a
measure must "effectively control access," so a **trivial or self-describing** format is the weak
case (arguably no circumvention required → not a TPM), while the **opaque, per-export-permuted,
schema-stripped, decode-compiled** protected mode is defensibly a TPM. The stronger and
less-trivial the measure (per-build permutation, no shipped schema, decode in compiled code /
shader / VRAM), the harder to argue it was not "really" a protection measure. So the technical
strength has direct legal weight.

**Honest caveats** (all reasons to document this as *mechanism*, not *guarantee*):
- **Jurisdictional** — anti-circumvention is broad but not uniform (the EU has interoperability
  carve-outs; some countries permit reverse-engineering for interoperability; enforcement differs).
- **Has exceptions** — statutory and periodic rulemaking exemptions exist (interoperability,
  security research, and for games notably preservation/mod exemptions). Strong, not absolute.
- **A legal position, not a technical guarantee** — neither the format nor the law *prevents*
  extraction; together they make it unlawful and enforceable. The "client holds the key" limit
  stands; the value is legal, not cryptographic.

**How to say it in loft's docs:** loft *"enables a proprietary-format TPM for asset sellers,"* not
loft *"prevents asset reuse."* The protection is mechanism + license + copyright together, and its
teeth are jurisdictional.

## What the freeze must reserve (the only pre-1.0 obligation here)

The mode itself is a future feature; the freeze's job is only to **not foreclose it**:

- **Freeze the LOGICAL layer** — the getter axiom's contract, field identity + order, enum ordering
  = declaration index, `==`/ordering semantics, the format caps.
- **Leave UNFROZEN** — the physical byte-encoding (offsets + discriminant values) *and* the decode
  location (CPU upload vs GPU shader). Both are implementations below the getter axiom, so
  per-export permutation and a later CPU→GPU decode move are legal *additive* changes post-1.0.

That single line — logical frozen, physical/decode free — is what keeps both the durable mmap store
and a future TPM-grade protected-asset mode alive past the freeze. It is recorded in
[formal-audit.md](formal-audit.md) § Ownership/heap/layout as the layout-freeze decision; this note
is its rationale.

## See also
- [formal-audit.md](formal-audit.md) — the layout-freeze decision (logical vs physical line).
- [GOALS.md](../../GOALS.md) / [BROADENING.md](../../BROADENING.md) — the platform-beyond-the-maker aim this serves.
- [plans/43-loft-store-durable/](../43-loft-store-durable/) — the durable mmap store (the canonical mode).
- [PACKAGES.md](../../PACKAGES.md) / [PKG_REGISTRY.md](../../PKG_REGISTRY.md) — where a license/EULA layer for sold assets would live.
