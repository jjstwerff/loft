<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN30 — Struct-enum + variant field-capture validation

**Status: CLOSED 2026-07-09 — delivered.**  loft's polymorphic-dispatch core
(variant-typed methods, `is` field-capture, `match` arms) is implemented and
heavily tested: ~135 `p54_*` / `plan19_*` / `n4_*` tests in `tests/issues.rs`
plus `tests/scripts/05-enums.loft`, `369-pln22-shared-enum-variants.loft`,
`406-enum-var-struct-field.loft`.  The one pre-flight gap (C5 — parent-enum
method dispatch, `s.classify()` on a variant value) was fixed in
`src/parser/fields.rs` and is pinned by
`tests/issues.rs::plan19_method_on_enum_variant_via_dot` (asserts
`Value::Float(12.56)`).  Phases 04–05 (the never-built `tests/struct_enum_matrix.rs`
binary) were demoted to documentation **per this plan's own pre-flight gate** —
no code work remains.

## Goal

Validate that struct-enum dispatch (variant-typed methods, `is`
checks, field capture, `match` arms) round-trips every meaningful
**variant payload type** through every meaningful **dispatch
context**, with **interp/native byte-identical stdout**.

This is loft's **polymorphic dispatch core**.  @P204 (tail-call
return through a struct-enum return type) and @P205 (text-returning
generic dispatch) both lived here.  The matrix pins the surface
that already absorbed two recent native-codegen P-issues.

## Why now — pre-flight survey

5 quick tests (2026-05-04), 1 fail.  The failure pattern is real:

| Shape | Result |
|---|---|
| Variant-typed method (`fn area(self: Circle)`) | ✅ |
| `is`-capture in `if` (`if s is Rect { w, h }`) | ✅ |
| `is` boolean check (`s is Circle`) | ✅ |
| Match dispatch on parent-enum-typed self | ✅ |
| Method on parent-enum type called via `.method()` on a variant value | ✅ (closed 2026-05-04 via parent-enum lookup in `parser/fields.rs::field`; only fires when the parent has a direct `fn …(self: Enum)` decl, never the auto-dispatcher built from per-variant impls — preserves the long-standing "Unknown field Variant.method" error when only a sibling variant has the impl) |

20% pre-flight rate (1/5) at survey time; rate now 0/5.  The
fix was a single localized parser-side dispatch fallback —
matches the pre-flight gate's prediction.

## The matrix

Two axes.

### Axis 1 — variant payload type

| ID | Payload | Notes |
|---|---|---|
| V0 | **No fields** — `enum E { A, B, C }` | Pure tag enum |
| V1 | **Single scalar field** — `Circle { radius: float }` | Pre-flight ✅ |
| V2 | **Single text field** — `Tag { name: text }` | Active risk: text element lifetime through dispatch (@P205 territory) |
| V3 | **Single Reference field** — `Wrapper { ref: Reference<S> }` | DbRef element |
| V4 | **Multi-field mixed** — `Rect { w: float, h: float }`, `Mixed { n: integer, s: text }` | Pre-flight ✅ for scalar; text-mixed less tested |
| V5 | **Tuple field** — `Pair { both: (integer, integer) }` | Cross-cuts @PLAN14 phase 05 (D3 struct field of tuple) |
| V6 | **Nested struct-enum** — variant payload contains another struct enum | Stretch |

### Axis 2 — dispatch context

| ID | Context | Notes |
|---|---|---|
| C1 | **`is` check** (boolean) | Pre-flight ✅ |
| C2 | **`is`-capture in `if`** | Pre-flight ✅ |
| C3 | **Match arm with capture** | Pre-flight ✅ |
| C4 | **Variant-typed method** (`fn area(self: Circle)`) | Pre-flight ✅ |
| C5 | **Parent-enum-typed method** (`fn classify(self: Shape)`) | Pre-flight ❌ when called on a variant value via method form |
| C6 | **Store as `Shape` then later read via match** | Polymorphic round-trip |
| C7 | **Return as parent-enum type** | Polymorphic return; cross-cuts @PLAN14 phase 04 (struct-ref tuple element) |

## Phase layout

| Phase | Variant rows | Context cols | Outcome |
|---|---|---|---|
| [00 — matrix freeze + harness wiring](00-matrix.md) | (table) | (table) | Frozen matrix; `tests/struct_enum_matrix.rs` binary; smoke test.  No production change. |
| 01 — V0/V1 baseline | V0, V1 | C1–C7 | Most should pass.  Establishes the harness shape against the simplest variants. |
| 02 — text payload (V2) | V2 | C1–C7 | Active risk: text lifetime through variant dispatch.  Cross-cuts @P205 territory. |
| 03 — fix C5 method resolution | V0–V4 | C5 | **Closed 2026-05-04.** Parser-side fix in `src/parser/fields.rs::field` adds a parent-enum method lookup (`t_<n>Parent_<field>`) when the receiver is `Type::Reference(child_d, …)` and `child_d`'s parent is an enum.  Guarded against the auto-generated polymorphic dispatcher (only fires when no sibling variant has a per-variant impl) so the "Unknown field Variant.method" error remains for unimplemented-variant calls.  Runs on both passes for first-pass type propagation.  Pinned by `tests/issues.rs::plan19_method_on_enum_variant_via_dot`. |
| 04 — multi-field + tuple payloads | V4, V5 | C1–C7 | Cross-cuts @PLAN14 phase 05; tuple-payload variants are the natural extension once tuple-in-struct-field is settled. |
| 05 — Reference + nested | V3, V6 | C1–C7 | DbRef payload is straightforward; nested struct-enum is the stretch. |
| 06 — freeze + doc | — | — | Update LOFT.md § Struct enums where the matrix surfaces under-documented behaviour. |

## Pre-flight gate

If phase 03 (C5 method dispatch) closes with a single small fix and
phases 01-02 pass mostly green, close phases 04-05 as deferred (the
matrix becomes documentation, not execution).

## Acceptance for the whole plan

- Matrix in [00-matrix.md](00-matrix.md) fully populated.
- C5 method-resolution gap closed.
- Every PASS cell has a `cross_mode!` test in
  `tests/struct_enum_matrix.rs`.

## Out of scope

- **Plain enum dispatch** (no variant fields) — already covered by
  existing tests and trivial; not matrix-shaped.
- **JSON round-trip of struct-enums** — covered by separate JSON
  tests; this matrix is dispatch-focused.
- **Polymorphic generic interaction** (`<T: Shape-like>`) — covered
  by @PLAN17.

## Cross-references

- [LOFT.md § Enums](../../LOFT.md) — language reference.
- `src/parser/control.rs` — match dispatch.
- [@PLAN14 phase 04](../finished/14-tuple-validation/04-references.md) —
  Reference tuples cross-reference.
- [@PLAN17](../finished/17-template-validation/README.md) — generic
  dispatch over Shape-like interfaces (closed 2026-05-09).
