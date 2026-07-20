<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 113 — Contract-keyed semantics (the edition-style escape valve)

## Status

Open — design ready, no implementation yet.  Tracks [`@PLN113`](https://github.com/loft-lang/plans/issues/113).
This builds the **contract-keyed escape valve** that [COMPATIBILITY.md § The escape valve for the
genuinely unavoidable](../../COMPATIBILITY.md) already declares as policy but leaves unbuilt (§ *Open
decisions* item 3: "designed when first genuinely needed, not before").  The **@PLN110 `len/size(text)`
flip** (merged #587) is that first genuine need and the defining instance.  Today the flip is a hard op
swap that stays compatible only because we are still pre-contract-1 (breaking is free before the
freeze); this plan turns it into a contract bump so it survives the freeze and never spends a "free
break."  Should land **by / with the contract-1 point release** — after the freeze, a `len/size`-style
change is *only* expressible through this mechanic.

## Goal

Let a source/package declare the `contract` it was authored against, and have a newer loft binary
**carry both semantics** and run old-contract code with its old meaning (AS/400 / edition-style) — so a
semantic language change lands with **no forced lib version bump and no existing program broken**.

## Effort + design

- **Effort:** MH–H (per-source contract threading + dual op variants on **both** backends + IR
  persistence + resolver matching).
- **Design:** ~ (partial — arcs and chokepoints identified; open questions below unresolved).
- **Last touched:** 2026-07-20

## What already exists (do not rebuild)

- **The `contract` integer + resolver check** — @PLN102 arc B.  `src/manifest.rs`:
  `package.contract: Option<String>` (declared predicate), `CONTRACT_VERSION: u32 = 0` (the contract
  the *binary* implements — currently 0, pre-freeze), `check_contract(required, current)` (`TooOld` /
  `Drifted` / `Ok`).  Resolver-side only; keys **no** semantics yet.
- **The owned-source author-alert gate** — @PLN102 arc C.  `src/keys.rs::steer_enabled()` +
  `DEF_SUPERSEDED` (`src/data_store.rs:313`, persisted through the IR store).  COMPATIBILITY.md: the
  escape valve "reuses this same owned-source gate but keys the author-alert to a `contract` bump."
- **The flip chokepoint** — `src/fill.rs:1140 length_text` / `:1146 size_text` (dispatch `:142-143`);
  stdlib surface `default/01_code.loft:675 OpLengthText` / `:685 OpSizeText`.

**The gap:** nothing threads a definition's declared contract to the op chokepoint, and the compiler
emits exactly one meaning for `len/size(text)`.  Filling it is the four arcs below.

## Composition matrix — Stage A

The spec.  This plan is done when **every cell is green on both backends** and the probes graduate to
`tests/scripts/113-contract-keyed-semantics.loft`.  Axes: **declared contract** × **operation** ×
**text content** × **backend**.

Both ops changed meaning (`phase0-inventory.md`: pre-flip `len(text)`=bytes / `size(text)`=chars;
post-flip `len`=chars / `size`=bytes), so **both** carry two variants.  `"café"` = 4 chars, 5 bytes:

| contract | content | expected `len` / `size` |
|---|---|---|
| 0 / undeclared | `"abc"` (ascii) | 3 / 3 |
| 0 / undeclared | `"café"` (1 multibyte) | **5 / 4** (pre-flip: `len`=bytes, `size`=chars) |
| N (flip) | `"abc"` | 3 / 3 |
| N (flip) | `"café"` | **4 / 5** (post-flip: `len`=chars, `size`=bytes) |

Each cell hand-computed (not two-binary agreement); run on `--interpret` **and** `--native`; the
contract-0 rows must be **byte-identical** to today's pre-flip output.  Resolver cell (arc D): a
`CONTRACT_VERSION=0` binary asked for a contract-N lib resolves to the last contract-0-compatible
version, never the N one.  Full design + the decisions this blocks on: [DESIGN.md](DESIGN.md).

## Sub-arcs

| Item | Concern | Status |
|---|---|---|
| **A** — Contract propagation + persistence | thread the declaring package's `contract` to each definition at compile; persist per-def (`DEF_CONTRACT` slot, mirroring `DEF_SUPERSEDED`); undeclared defaults to **oldest** (frozen old behavior) | Open |
| **B** — Compile-time op selection + flip-as-bump | keep both variants (`length_text` + `length_text_v0`, size counterparts) in `src/fill.rs` **and** `src/generation/`; **codegen** picks by caller contract; bump `CONTRACT_VERSION` so new `len`=chars is the contract-N behavior | Open — **critical path** |
| **C** — Author steering keyed to the bump | owned source on the old variant → optional steer to declare the newer contract, via `steer_enabled()` + owned-source gate; keyed to a `contract` bump, not a bare `#superseded` | Open |
| **D** — Resolver contract-matching for libs | `loft install` resolves the newest lib version whose `contract` predicate the binary's `CONTRACT_VERSION` satisfies → an old binary never pulls a new-contract lib | Open — closes the version-forcing loop |

## Phase ordering

1. **A** — propagation + persistence (nothing keys without it).
2. **B** — op selection + `CONTRACT_VERSION` bump.  **This alone lands the @PLN110 flip safely** — it
   is the critical path; the Stage-A matrix is its acceptance gate.
3. **C** — steering (inert until the contract bumps; ergonomics polish).
4. **D** — resolver matching.  The arc that actually deletes "forced to make versions"; needed before a
   new-contract lib is published so old binaries stay on the compatible line.

## Open design questions

1. **Manifest-less source (bare scripts / REPL) default contract.**  Oldest-frozen (an old re-run is
   unchanged, but a *new* script must opt in for new `len`) vs. latest (convenient, but an old bare
   script silently changes under a new binary).  Editions default to oldest; loft's no-manifest surface
   makes this genuinely ambiguous.  Resolution → DESIGN_DECISIONS.md.
2. **Contract number mapping.**  Is the flip *contract 1* (contract-1 = new `len` **and** the freeze in
   one milestone) or a later contract?  Mechanism identical; only the integer differs.
3. **Do we ever drop the oldest variant (fold a contract)?**  Never-remove + the registry scan proves
   only *public* usage (private programs invisible ⇒ zero unprovable) ⇒ likely **carry both forever**
   (bounded, permanent cost).  State it explicitly.
4. **Granularity.**  Package-level `contract` only, or also a per-file pragma?

## Cross-arc dependencies

- **@PLN102** — sits on arc A (COMPATIBILITY.md, this escape-valve section), arc B (the `contract`
  integer + `check_contract`, reused by arcs A/D), arc C (the owned-source steer gate, reused by arc C).
- **@PLN110** — first customer; this plan is how that flip survives the contract-1 freeze instead of
  spending loft's last free break.

## See also

- [COMPATIBILITY.md § The escape valve for the genuinely unavoidable](../../COMPATIBILITY.md) — the
  policy this implements; [§ Open decisions](../../COMPATIBILITY.md) item 3 links here.
- [plans/102-stability-contract/README.md](../102-stability-contract/README.md) — @PLN102 (arcs A/B/C
  this builds on).
- [plans/110-len-size-semantics/](../110-len-size-semantics/) — @PLN110, the flip that motivates this.
- [`@PLN113`](https://github.com/loft-lang/plans/issues/113) — the tracker issue.
