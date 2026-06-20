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
