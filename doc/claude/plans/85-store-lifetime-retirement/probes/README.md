<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Probes — @PLN85 store-lifetime retirement

Stage A probes go here, one assertion-bearing `.loft` file per shape
(`NN-<slug>.loft`). Seed from the four repros (#405/#406/#409/#410) +
a real-consumer FFI-return extraction; run each on `--interpret` AND
`--native`; record the matrix in the parent README's probe table.

A probe graduates to `tests/scripts/85-<slug>.loft` only when it passes:
assertions · clean process exit · no leak (`LOFT_STORES=warn`) · bounded
runtime. See [`../README.md`](../README.md) and the investigation template.

## Crawler dogfood wave (2026-06-25) — #462 cluster + sibling sweep

Driven by the crawler consumer; full analysis in
[`../cluster-462-slot-reuse-uaf.md`](../cluster-462-slot-reuse-uaf.md).

| Probe | Shape | interp | native | Note |
|---|---|---|---|---|
| `462-nullable-append-clean.loft` | `vector<__nullable<S>>` += `[fn(structarg)->33-field struct]` loop | ✅ | ✅ | control — #462 shape, clean in isolation |
| `462-borrowed-element-return-clean.loft` | + source = borrowed nullable-vec element (`mon_choose_habitat` shape) | ✅ | ✅ | control |
| `sib-457-struct-return-arms.loft` | STRUCT (not vector) adopted across if/else arms + churn | ✅ | ✅ | #457 struct analog — clean |
| `sib-457-match-vector-return.loft` | vector return across a 3-arm `match` + churn | ✅ | ✅ | clean |
| `sib-457-nested-vector-return-arms.loft` | `vector<vector<integer>>` delivered across arms + churn | ✅ | ✅ | nested delivery clean |
| `sib-462-nullable-struct-append-churn.loft` | #462 shape + interleaved churn (force slot reuse) | ✅ | ✅ | clean — minimal scale doesn't trip the UAF |
| `sib-nullcoalesce-nested-len.loft` | `len(vv[i] ?? [])` — vector-literal default on a nested-vector element | ✅ | 🔴 **E0308** | **STILL FAILS (native)** — divergence |
| `sib-nullcoalesce-nested-bind.loft` | `x = vv[i] ?? []` — same, bound to a var | 🔴 **panic** | 🔴 **panic** | **STILL FAILS (both)** — `codegen.rs` slot assert |

### Full field map (~95 shapes) — see [`../nullable-materialization-field-map.md`](../nullable-materialization-field-map.md)

A generator sweep (`gen_sweep.py`, session scratchpad) over the construction-path ×
element-type × default-kind × use-context × delivery × backend cross-product found the
failures cluster into **two crisp families** (plus #462's slot-reuse class). All
delivery (D-*) and append (P-*) shapes are clean at minimal scale.

| Family probe | Family | Trigger | interp | native |
|---|---|---|---|---|
| `sib-nullcoalesce-nested-len.loft` | **A** `?? <vec-literal>` | `len(vv[i] ?? [])` | ✅ | 🔴 E0308 |
| `sib-nullcoalesce-nested-bind.loft` | **A** | `x = vv[i] ?? []` | 🔴 panic | 🔴 panic |
| `46A-coalesce-veclit-nonempty.loft` | **A** | `?? [99]` (non-empty, any context) | 🔴 panic | 🔴 panic |
| `46A-coalesce-vstruct-veclit.loft` | **A** | `len(vv_of_struct[i] ?? [])` | ✅ | 🔴 E0308 |
| `46N-litelem-fncall-promote.loft` | **N** literal-elem promote | `[mk_s(1)]` → `vector<__nullable<S>>` | 🔴 compile | 🔴 compile |
| `46N-litelem-ternary-promote.loft` | **N** | `[if c {…} else {…}]` → promoted | 🔴 compile | 🔴 compile |

- **Family A** — a **vector-literal** coalesce default (`?? []`, `?? [99]`) is never
  materialised in the else-branch (native `{ncc} else {()}` E0308; interp slot panic).
  Clean siblings: `?? scalar`, `?? fn()`, `?? element`, `?? struct-literal`.
- **Family N** — a vector literal whose element is **anything but a direct
  struct-literal** (fn-call, field-read, ternary, coalesce) is over-promoted to
  `vector<__nullable<S>>` — element nullability is decided on syntactic literal-ness,
  not the element's type.

Both are crisp + minimal (unlike #462). The field-map doc has the per-cell complexity
tables and the **unifying pattern** (a materialisation hole for freshly-constructed
vector/struct values at the nullable boundary).
