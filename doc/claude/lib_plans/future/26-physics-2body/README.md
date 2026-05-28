<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# lib-plan 26 — `physics_2body`: shared rigid-body physics primitives

**Status:** FUTURE (slot created 2026-05-28).  Driven by the
cross-project consumer audit in
[lib-plan 12 § Cross-project consumers](../../12-library-extraction/README.md#cross-project-consumers--moros--dryopea--bumper-airplanes):
three games (moros, [@PLAN46 dryopea](../../../plans/future/46-dryopea/README.md),
[@PLAN50 bumper-airplanes](https://github.com/jjstwerff/loft/tree/bumper_plane/doc/claude/plans/future/50-bumper-airplanes))
all need 2-body rigid collision + integrator code; without a shared
package, PLAN50 ships bespoke physics and the others copy-and-modify.

This slot exists to design the API **before** PLAN50 starts coding it.

## Why "2-body"

The shared core is **pairwise** rigid-body interaction:
- One body vs geometry (sphere/AABB → static mesh collision).
- One body vs one body (sphere/AABB → sphere/AABB collision).

Multi-body resolution (N planes settling against each other in a
pile) is **out of scope**; bumper resolves at most one pair per
physics tick, dryopea's vehicle interacts with at most a few enemies
at a time, moros's player interacts with one wall at a time.  The
"2-body" name documents the constraint — if true rigid-body piles
ever become a requirement, that's a different library.

## Consumer surface (matrix)

| Consumer | Bodies | Geometry | Why this library |
|---|---|---|---|
| **moros** | Player + simple enemies (2D-ish) | Hex walls + ramps from `lib/world` | Generalises `lib/moros_sim/collide.loft` (currently moros-internal); same surface, more dimensions |
| **dryopea** | Hover vehicle, enemy units, scramble rocket | Terrain heightfield + built walls + boss-broken segments | Vehicle motion + enemy collisions + ballistic rocket all need the same integrator |
| **bumper** | Plane (sphere collider), bumper targets, peer planes | Extruded palette terrain (pillars, cliffs, ramps) | Plane bounce + plane-on-plane (sphere-sphere) + target bounce (sphere-sphere with reflection energy split) |

## Proposed API (sketch — not locked)

### Types

```loft
// Pose: position + orientation + linear/angular velocity.
pub struct Body {
  pos:       Vec3,
  orient:    Quat,
  vel:       Vec3,
  ang_vel:   Vec3,
  mass:      float,
  inertia:   float,   // scalar — bodies are sphere-equivalent for inertia
  shape:     Shape,
}

// Collider shape.  Spheres are the universal case; AABB for terrain
// tiles; mesh-triangle for arbitrary geometry.
pub enum Shape {
  Sphere(radius: float),
  Aabb(half_extents: Vec3),
  Mesh(ref: MeshHandle),   // mesh lives in lib/world or lib/graphics
}

// One pair of bodies in contact this tick.
pub struct Contact {
  a_idx:        integer,
  b_idx:        integer,   // -1 if static-geometry hit
  point:        Vec3,
  normal:       Vec3,   // points from b toward a
  penetration:  float,
}
```

### Step function

```loft
// Integrate one tick.
//   bodies        : in/out; positions + velocities updated.
//   static_geom   : query interface (raycast / closest-point);
//                   typically a lib/world Chunk + lib/gridmesh BVH.
//   dt_secs       : tick duration (typically 1/60 .. 1/30).
//   restitution   : per-pair energy retention [0, 1]
//                   (sphere-sphere ~0.4, sphere-geom ~0.4,
//                    sphere-target ~0.7 outward / 0.2 tangent).
//   gravity       : Vec3 (per-game; bumper uses 0, dryopea uses -9.8 y, moros uses 0)
// Returns: contacts resolved this tick (for game-layer score events).
pub fn step(
  bodies:       vector<Body>,
  static_geom:  &GeomQuery,
  dt_secs:      float,
  restitution:  RestitutionFn,
  gravity:      Vec3,
) -> vector<Contact>
```

### Reflection rule

```loft
// Bumper-style: tangential / normal energy split.
pub fn reflect(v: Vec3, normal: Vec3, e_normal: float, e_tangent: float) -> Vec3
```

PLAN50's "0.7 outward / 0.2 tangent" target-hit kick is `reflect(v,
n, 0.7, 0.2)`.  Default sphere-vs-geometry bounce is `reflect(v, n,
0.4, 0.9)` (slight tangential friction).

### Event callbacks

The step returns the contact list; the caller fires score events,
stall timers, smoke "puffs," etc. from there.  Physics has no
knowledge of game scoring — see PLAN50's nose-vs-body scoring rule,
which lives in the consumer.

## What's NOT in this library

- **Pile-of-rigid-bodies stacking** (N-body iterative resolution).
  Bumper's worst case is one pair per tick.
- **Soft-body** / **cloth** / **fluid**.  Out of scope.
- **Continuous collision detection** beyond 2-body sphere swept tests.
  Discrete tick + sphere swept is enough for the three consumers.
- **Constraint joints** / **inverse kinematics**.  No game listed
  needs them.
- **Game-specific scoring** (nose vs body, target cooldown).
  Stays in the consumer.

## Implementation phases (sketch)

| # | What ships | Effort | Notes |
|---|---|---|---|
| 1 | Types + reflection helper + sphere-vs-AABB step | XS | Closes 80 % of moros's `collide.loft` use cases |
| 2 | Sphere-vs-Mesh via `lib/gridmesh` BVH | S | Required for dryopea terrain + bumper extruded palette |
| 3 | Sphere-vs-sphere (plane-on-plane) | XS | PLAN50's body-vs-nose 2-collider pattern |
| 4 | Tunable restitution per-pair (`RestitutionFn`) | XS | Lets PLAN50's target-kick differ from generic bounces |
| 5 | Stall / dampening hooks (per-body control-authority multiplier with decay) | S | PLAN50 stall-mode pattern; reusable for dryopea boss-stuns |

## Open questions

1. **Integrator:** semi-implicit Euler (cheapest, "good enough for
   games") vs RK4 (more stable)?  Bumper's plane physics may need
   RK4 for snappy bank-roll; moros's player doesn't care.  Probably
   semi-implicit by default, RK4 opt-in.
2. **Coordinate convention:** y-up or z-up?  `lib/world` is z-up
   (hex addressing); `lib/graphics` math is y-up.  Pick one and
   document.
3. **Tick coupling:** does the library own its own tick rate, or
   does the consumer step it at game tick rate?  Game-driven is
   simpler; library-driven would let physics + game ticks diverge.
   Probably game-driven.
4. **WASM**: do all integration paths work under wasm32-wasip2 +
   browser, or do we need a separate path?  Physics is pure math —
   should be portable, but `f32` precision differences across
   targets need verifying.

## Cross-references

- [lib_plans/12-library-extraction](../../12-library-extraction/README.md)
  — Phase 7p of that plan blocks on this slot existing.
- [@PLAN46 dryopea](../../../plans/future/46-dryopea/README.md) —
  consumer for vehicle + enemies + scramble rocket.
- [@PLAN50 bumper-airplanes](https://github.com/jjstwerff/loft/tree/bumper_plane/doc/claude/plans/future/50-bumper-airplanes)
  — consumer; sub-arc 4 originally proposed this library.
- `lib/moros_sim/collide.loft` — existing 2D-ish code that becomes
  Phase 1's initial population once this slot starts.
