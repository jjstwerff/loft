# Phase 3 — C54.C: add `u32` as a stdlib type

Status: **not started** — blocked by Phase 2.

## Context

Post-C54.A, `u32` is trivially expressible.  Per QUALITY.md § 442-453:

```loft
pub type u32 = integer limit(0, 4_294_967_294) size(4);
```

The sentinel reservation (one short of 2³²) matches the existing
`u8 = integer limit(0, 255) size(1)`.  Users needing the exact top
value write `u32 not null`.

Closes the "RGBA pixels wrap negative" trap — without `u32`, users
currently hit the `i32::MAX` boundary in pixel math and the result
silently wraps through the null sentinel.

## Scope

- Add the `type` alias to `default/01_code.loft` (or the appropriate
  stdlib prelude file).
- Ensure `limit(0, 4_294_967_294)` is accepted by the typedef parser
  at those bounds (may already be; verify).
- Export as a public type.
- Document in LOFT.md type-reference section.

## Test plan (from QUALITY.md § 452-453)

Un-ignore:
- `c54c_u32_rgba_round_trip`
- `c54c_u32_arithmetic_promotes`
- `c54c_u32_not_null_full_range`
- `c54c_u32_size_is_4`

Plus a moros_render / graphics probe: RGBA pixel math on a small
image shows correct values at all boundary pixels.

## Budget

**60-120 minutes**.  This is the smallest phase.  If it isn't trivial
after Phase 2, something has gone wrong.

## Deliverables

- `pub type u32 = …` in stdlib.
- Tests un-ignored.
- LOFT.md updated.
