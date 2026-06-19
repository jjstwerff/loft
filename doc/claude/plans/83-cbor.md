<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN83 — `cbor` library: signable canonical CBOR encoding

Status: **future** (scoped, not started) · Issue: [loft-lang/plans#83](https://github.com/loft-lang/plans/issues/83) · Subject: libs

A loft library that encodes/decodes loft data ↔ RFC 8949 CBOR bytes, with a
**canonical (deterministic) mode** as the first-class signable path.

## Driver

The **zero-trust-shared-files** consumer (`/home/jurjen/workspace/zero-trust-shared-files`,
its `DEPENDENCIES.md`) marks `cbor` the **critical-path** new library: signable binary
records, op envelopes, and CRDT snapshots need a compact, deterministic, binary-safe
encoding so signatures are stable across implementations and the wire format can freeze.
Interim guidance from that consumer: prototype records over the stdlib JSON now, but **do
not freeze the wire / signature format on JSON** — it must be CBOR-canonical from the first
frozen version.

## The invariant (load-bearing)

*Encode is a pure, canonical function of the value: the same logical value produces
byte-identical output, across implementations AND across targets (native ↔ wasm).* A
single non-canonical map ordering silently breaks every signature, so this is a
test-vector / matrix problem, not a "looks right" one.

## Why CBOR over the stdlib JSON here

- **Native byte strings** (major type 2) — crypto records are mostly raw bytes (keys,
  ciphertext, sigs, nonces); JSON forces base64 (≈33% bloat + ambiguity).
- **A canonical form** (RFC 8949 §4.2) — JSON has none; key order and number formatting
  drift, so JSON signatures aren't stable.
- **Compact binary wire** that can freeze.

## Structure

Mirrors the crypto `[native]`-crate template, with two deliberate departures:

- `cbor/{loft.toml, src/cbor.loft, native/, tests/}` over the upstream `ciborium` crate.
- **Modern `#[loft_native]` bridge passing `vector<u8>` directly** — *not* the legacy
  base64-text-over-C-ABI convention the crypto v0.2 delta still uses (cbor is binary; a
  base64 round-trip would defeat the point).
- **No `[wasm.bridge]`, no JS.** `ciborium` is pure Rust → cross-builds to a wasm32 rlib
  directly. This is the big simplification vs crypto (which needs `crypto.subtle`): cbor is
  platform-blind for free.

## API surface — parallels loft's existing JSON

| JSON (exists) | `cbor` (mirror) |
|---|---|
| `T.to_json()` / `Type.parse(text)` | **typed:** `T.to_cbor()` / `parse_cbor<T>(bytes)` — reuse the struct↔value machinery behind `to_json` |
| `JsonValue` + `json_parse` / `to_json` | **dynamic:** `CborValue` enum (`CInt`/`CBytes`/`CText`/`CArray`/`CMap`/`CBool`/`CNull`/`CFloat`/`CTag`) + `cbor_encode`/`cbor_decode` |
| — | **canonical:** `to_cbor_canonical()` / `cbor_encode_canonical()` — the signable path |

**v1 scope:** core data items (uint/negint, byte string, text string, array, map, bool,
null, float64), both paths, canonical mode — enough for signable records/ops/snapshots.
**Deferred:** rich tags, indefinite-length, bignums, half/single-float *encode*
(decode-tolerant), CBOR sequences (RFC 8742).

## Decisions to settle in-phase

1. **Canonical ordering is real work — `ciborium` doesn't give it.** It emits shortest-form
   ints + definite-length but **preserves serde field order, not byte-sorted keys**. The
   lib must canonicalize itself: build a `ciborium::value::Value`, recursively sort map keys
   by encoded bytes, then serialize. This is C3 and the core risk.
2. **Map key type.** Support both text and integer keys; canonical-encode sorts regardless.
   Document **integer keys (COSE-style)** as the recommended pattern for signable records.
3. **Typed field order.** A loft struct's fields are in declaration order, not canonical
   order → the signable path is `to_cbor_canonical` (sorts); plain `to_cbor` may preserve
   field order for speed. Anything signed round-trips through canonical.
4. **The linchpin:** confirm loft `vector<u8>` ↔ CBOR byte-string marshals cleanly over the
   modern bridge (the whole reason to avoid base64).

## Phases

- **C1 — scaffold** (`loft new cbor --native`, wrap `ciborium`, the byte-vector
  `#[loft_native]` signature). — S
- **C2 — dynamic** `CborValue` + `cbor_encode`/`cbor_decode` over `ciborium::value::Value`.
  — S
- **C3 — canonical encoding** (recursive map-key sort + shortest-form/definite-length
  guarantee + RFC 8949 §4.2 test vectors + an encode→decode→encode byte-identity corpus).
  — **M, the load-bearing part**
- **C4 — typed path** `T.to_cbor()` / `parse_cbor<T>` reusing loft's struct↔value machinery
  (parallel to `to_json`), canonical variant. — M
- **C5 — wasm** cross-build the bridge crate to a wasm32 rlib (no JS bridge) + a
  **native-vs-wasm byte-equality test** (cross-target determinism = the signature-stability
  proof). — S
- **C6 — publish** native + wasm artifacts, registry entry, LIBRARY_CHECKLIST. — S

**Effort ~M.** C3 + C5 carry the risk and are exactly where a signature scheme is brittle,
so they are test-vector / matrix-driven, not eyeballed.

## See also

- [PKG_REGISTRY.md](../PKG_REGISTRY.md) — packaging + registry; this lib publishes through it.
- loft's JSON impl (`src/json.rs`, `src/native.rs` `to_json`/`json_parse`) — the in-tree
  architectural template the API mirrors.
- `crypto` `[native]`-crate package — the packaging/bridge structural template.
