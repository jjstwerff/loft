<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN83 — `cbor` library: signable canonical CBOR encoding

Status: **finished** (shipped — `cbor` published to the registry, 0.1.0/0.1.1; issue #83 closed 2026-07-07) · Issue: [loft-lang/plans#83](https://github.com/loft-lang/plans/issues/83) · Subject: libs

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

## Architecture — PURE LOFT (revised 2026-06-19, validated)

**Pivot from the original "wrap `ciborium`" plan** (and from `DEPENDENCIES.md`'s
suggestion). Investigating the loft-ffi bridge showed: the modern `#[loft_native]` bridge
can pass `vector<u8>` (via `LoftStore::alloc_vector` / `vector_data_ptr`), but a
*tree-walking* codec (a `CborValue`/struct → bytes) would force the native side to read
loft's struct layout by offset — re-introducing the **H9 cross-binary layout-coupling
hazard** ([STABILITY_HOTSPOTS.md § H9](../STABILITY_HOTSPOTS.md)), which is the wrong risk
for a signature-bearing security lib.

CBOR's byte format is simple (a major-type byte + length + payload), so the codec is
written **in pure loft** instead:

- `cbor/{loft.toml, src/cbor.loft, tests/*.loft}` — **no `native/`, no `ciborium`, no
  bridge.** Just loft.
- **`native == wasm` for free** — pure loft runs identically on every target, so the
  master invariant (the signature-stability proof) holds by construction, not by a
  cross-build equality test.
- **No external-crate trust surface** — the codec is auditable loft, and loft's memory
  safety makes a malformed-CBOR decode a *parse error*, not a crash (safer than a C/Rust
  parser). The mild "don't roll your own serialization" caveat is answered by testing
  against the RFC 8949 §Appendix-A vector corpus.

**Validated (probe, 2026-06-19):** a pure-loft encoder produced byte-identical RFC 8949
output for uint (`1000`→`19 03 e8`), text (`"IETF"`), byte string (`h'01020304'`), and
array (`[1,2,3]`). loft's `vector<u8>` + `+= [x as u8]` + integer arithmetic for the
big-endian length encode is the whole substrate; the narrow-int check (`as u8` required)
guards every byte.

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

## Phases (pure-loft — no native crate, no cross-build)

- **C1 — skeleton + primitive encoders.** `loft.toml` + `src/cbor.loft` + `tests/`; the
  `CborValue` enum; canonical encode for uint / negint / byte-string / text-string / bool /
  null / float64 (shortest-form head). *Check:* RFC 8949 Appendix-A primitive vectors
  byte-identical. **(core already validated by the probe.)**
- **C2 — containers + canonical map ordering.** array, map, nesting; **map keys sorted by
  encoded bytes** (RFC 8949 §4.2). *Check:* container vectors + a key-ordering vector (keys
  inserted out of order encode sorted). — **the load-bearing part**
- **C3 — decode** (bytes → `CborValue`) with well-formedness checks (loft-safe: malformed →
  error, never a crash). *Check:* decode the RFC corpus; `encode→decode→encode`
  byte-identity; negative tests reject truncated / non-canonical / overlong inputs.
- **C4 — typed path** `T.to_cbor()` / `parse_cbor<T>`: loft struct ↔ CBOR map (int or text
  keys), canonical. *Check:* struct corpus round-trips; signed-record shape stable.
- **C5 — package + publish.** `loft.toml`, registry entry, LIBRARY_CHECKLIST; a guard test
  asserting `native == wasm` output (trivial for pure loft, but pinned). *Check:*
  `loft install cbor` works; checklist green.

**Effort now ~S–M** (smaller than the ciborium-wrap plan — no native crate, no
cross-build). Risk concentrates in **C2** (canonical map ordering) and **C3** (robust
decode), both vector / negative-test driven.

## See also

- [PKG_REGISTRY.md](../PKG_REGISTRY.md) — packaging + registry; this lib publishes through it.
- loft's JSON impl (`src/json.rs`, `src/native.rs` `to_json`/`json_parse`) — the in-tree
  architectural template the API mirrors.
- `crypto` `[native]`-crate package — the packaging/bridge structural template.
