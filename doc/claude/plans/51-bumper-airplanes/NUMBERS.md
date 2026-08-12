<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN51 — Tunable parameters

Every value in the design is tuned, not derived.  This document
collects them in one place so playtest can sweep them without
hunting through prose.  Mirrors dryopea's
[`docs/NUMBERS.md`](https://github.com/jjstwerff/dryopea/blob/main/docs/NUMBERS.md) +
[`examples/numbers.json`](https://github.com/jjstwerff/dryopea/blob/main/examples/numbers.json)
pattern: this Markdown carries the **rationale**; the runtime
demo will load values from a JSON file (likely
`tools/audience-demo-50/numbers.json` once phase 0 lands).

**Format:**
- **DEFAULT** is a starting value, not a final value.
- **RANGE** is the playtest-sweepable window where the value is
  expected to still make sense.  Outside the range the design
  may need to change, not just the number.
- **WHY** is the design pressure on the value — what gets worse if
  you raise it, what gets worse if you lower it.

Where a parameter is referenced in [`README.md`](README.md) by a
specific number, that number is just illustrative.  This file is
canonical.

---

## Plane physics

| Parameter | Default | Range | Unit | Why |
|---|---|---|---|---|
| `plane.cruise_speed` | 20 | 12 – 30 | m/s | The reference airspeed for "level flight, both thumbs centred."  Faster = more chaos, less control; slower = sluggish and the engine drone feels weak |
| `plane.climb_rate_max` | 6 | 3 – 12 | m/s | Vertical velocity at full `pitch_input = +0.5`.  Too high = trivial to climb over everything; too low = bottom-locked |
| `plane.dive_rate_max` | 18 | 10 – 30 | m/s | Vertical velocity at full `pitch_input = -0.5`.  Asymmetric vs climb because diving converts altitude into speed |
| `plane.roll_rate_max` | 90 | 45 – 180 | deg/s | Bank rate at full `roll_input = ±1`.  Too high = twitchy; too low = banked turns are huge arcs |
| `plane.yaw_rate` | 30 | 15 – 60 | deg/s | Constant yaw rate while in yaw mode (one thumb in contact).  "Slow" relative to roll-based turns; should not replace banked turns for sharp manoeuvres |
| `plane.collider.nose_radius` | 0.5 | 0.3 – 0.8 | m | Sphere radius for the red-nose-only collider.  Too small = anti-coordination rule rarely fires; too large = it's almost the whole plane and side-attacks become impossible |
| `plane.collider.body_radius` | 1.5 | 1.0 – 2.5 | m | Sphere radius for the main-body collider.  Too small = misses look skipped; too large = invisible-wall feel |
| `plane.collider.nose_offset` | 1.2 | 0.8 – 2.0 | m | Distance from plane centre-of-mass to nose-sphere centre, along forward axis |

## Bounce + stall

| Parameter | Default | Range | Unit | Why |
|---|---|---|---|---|
| `bounce.geometry.energy_retain` | 0.4 | 0.2 – 0.7 | (unitless) | Velocity-magnitude scale after a cliff/pillar/ground bounce.  Too high = planes ping around forever; too low = bounce feels like a brick wall |
| `bounce.target.radial_retain` | 0.7 | 0.5 – 0.95 | (unitless) | Velocity along surface normal preserved (then reversed) on a target hit.  Drives the strong "kick" feel |
| `bounce.target.tangent_retain` | 0.2 | 0.0 – 0.5 | (unitless) | Velocity along surface tangent preserved on target hit.  Low = no skimming |
| `bounce.plane_plane.elasticity` | 0.5 | 0.3 – 0.8 | (unitless) | Combined energy retention for plane-on-plane (equal mass).  Affects how dramatic plane-on-plane collisions are |
| `stall.duration` | 3.0 | 1.5 – 5.0 | s | Time after a plane-on-plane bounce during which control is dampened.  Long = punishment; short = no consequence |
| `stall.control_damp` | 0.3 | 0.1 – 0.6 | (unitless) | Multiplier on pitch/roll/yaw input during stall.  Lower = more out-of-control |
| `stall.random_impulse_mag` | 90 | 30 – 180 | deg/s | Magnitude of the random angular impulse applied at bounce-instant (one-shot, decays naturally — NOT a sustained force).  Sets how "thrown" the plane feels |
| `stall.angular_drag` | 0.6 | 0.3 – 1.0 | per second | Natural angular damping during stall; brings the random tumble back to controllable |

## Targets (red/white bull's-eye spheres)

| Parameter | Default | Range | Unit | Why |
|---|---|---|---|---|
| `target.diameter` | 2.0 | 1.0 – 3.0 | m | Sphere diameter.  Too small = hard to spot at projector distance; too large = trivial to hit |
| `target.cooldown_secs` | 15.0 | 5.0 – 30.0 | s | Time after hit during which target is tilted-down + pass-through.  Long = forces map exploration; short = farming locally is viable |
| `target.reprime_animation` | 0.3 | 0.15 – 0.6 | s | Duration of the tilt-back-to-upright animation when the cooldown ends.  Just long enough to read as "I'm back" |
| `target.tilt_angle` | 70 | 45 – 90 | deg | How far the target rotates from upright when spent.  Bigger = more obviously "spent" at distance |
| `target.score` | 1 | 1 – 3 | points | Score per target hit.  Increase if target-farming feels under-rewarded relative to plane-hits |

## Plane-on-plane scoring

| Parameter | Default | Range | Unit | Why |
|---|---|---|---|---|
| `score.plane_hit` | 5 | 3 – 10 | points | Score for a body-hit on another plane.  +5 vs +1 target ratio sets the "is it worth chasing players?" balance |
| `score.combo_window` | 0 | 0 – 8 | s | Optional: hits within window multiply.  0 = no combos.  See open question on combo system |
| `score.combo_max_mult` | 1 | 1 – 5 | × | Max combo multiplier reached at end of chain.  1 = no combos |

## Smoke trail + confetti

| Parameter | Default | Range | Unit | Why |
|---|---|---|---|---|
| `trail.length_secs` | 3.0 | 1.5 – 6.0 | s | How long a smoke trail persists.  Longer = more visual clutter; shorter = harder to track who's where |
| `trail.sample_rate` | 30 | 20 – 60 | Hz | How often a new trail point is recorded.  Lower = chunky trail; higher = smooth |
| `trail.ring_size` | 90 | 60 – 240 | entries | `trail.length_secs × trail.sample_rate`.  Pre-allocated per-plane buffer |
| `trail.alpha_at_head` | 1.0 | 0.7 – 1.0 | (unitless) | Trail alpha at the plane (most recent point) |
| `trail.alpha_at_tail` | 0.0 | 0.0 – 0.3 | (unitless) | Trail alpha at the oldest point |
| `confetti.burst_count_p1` | 30 | 15 – 80 | particles | Confetti particle count for a +1 target hit |
| `confetti.burst_count_p5` | 80 | 40 – 200 | particles | Confetti particle count for a +5 plane hit |
| `confetti.duration` | 1.0 | 0.6 – 2.0 | s | How long the confetti burst takes to fade |
| `confetti.gravity` | 4.0 | 0.0 – 10.0 | m/s² | Downward acceleration on confetti.  0 = static cloud; high = quick fall |

## Sound

| Parameter | Default | Range | Unit | Why |
|---|---|---|---|---|
| `audio.engine_pitch_min` | 0.7 | 0.5 – 1.0 | × ref | Engine drone pitch at minimum airspeed |
| `audio.engine_pitch_max` | 1.6 | 1.2 – 2.4 | × ref | Engine drone pitch at maximum airspeed |
| `audio.engine_volume` | 0.4 | 0.0 – 1.0 | (unitless) | Engine drone volume relative to event sounds.  Low so events stand out |
| `audio.bounce_geometry_volume` | 0.8 | 0.0 – 1.0 | (unitless) | Volume of geometry-bounce *bonk* |
| `audio.bounce_target_volume` | 0.9 | 0.0 – 1.0 | (unitless) | Volume of target-hit *chime* |
| `audio.bounce_plane_volume` | 1.0 | 0.0 – 1.0 | (unitless) | Volume of plane-on-plane *clang* (loudest because it has stall consequences) |
| `audio.stall_volume` | 0.5 | 0.0 – 1.0 | (unitless) | Stall-whirr volume, fades to 0 over `stall.duration` |
| `audio.score_volume` | 0.6 | 0.0 – 1.0 | (unitless) | Score *ding* volume |

## Controls

| Parameter | Default | Range | Unit | Why |
|---|---|---|---|---|
| `controls.mode_transition_hysteresis` | 150 | 50 – 400 | ms | Required stable single-thumb-on-screen duration before yaw mode engages.  Filters out capacitive-touch dropouts |
| `controls.thumb_deadzone` | 0.05 | 0.01 – 0.15 | (fraction of strip) | Thumb-position window around 0.5 that reads as "centred."  Avoids drift-jitter |

## Networking

| Parameter | Default | Range | Unit | Why |
|---|---|---|---|---|
| `net.input_rate` | 30 | 20 – 60 | Hz | How often a phone sends its `(L, R, L_on, R_on)` snapshot to the server |
| `net.broadcast_rate` | 30 | 15 – 60 | Hz | How often the server sends per-plane poses to all clients |
| `net.player_cap` | 30 | 6 – 30 | (count) | Hard cap on simultaneous players.  Beyond this, joiners spectate |
| `net.peer_sight_range` | 80 | 30 – 200 | m | World-space radius around each phone's own plane within which OTHER planes' poses are broadcast to that phone.  Drives the dominant throughput term: when range ≪ map diagonal, most planes are filtered out per-recipient, so per-phone outbound bandwidth scales with **typical visible-peer count** rather than (N − 1).  Smaller = lighter network + tighter cockpit-view focus; larger = more situational awareness but more bandwidth and more first-person clutter.  Projector receives ALL planes unconditionally (the projector is the overview by design — never sight-filtered) |
| `net.pose_frame_bytes_budget` | 32 | 16 – 96 | bytes | Per-plane pose-frame size budget on the wire.  Position (3 × i16 fixed-point ≈ 6 B), orientation (1 × u16 yaw + 1 × u16 pitch + 1 × u16 roll ≈ 6 B), velocity (3 × i16 ≈ 6 B), stall+cooldown flags (2 B), seq + plane_id (4 B), padding ≈ 32 B.  Floats blow this up; fixed-point keeps the pump under typical pose-traffic budgets |
| `net.peer_rate_full_radius` | 25 | 10 – 50 | m | Inner rate-LOD ring.  OTHER planes within this radius of a phone's own plane get **every** broadcast tick — full `net.broadcast_rate` updates.  This is the "knife-fight" range where every twitch matters and interpolation can't hide latency |
| `net.peer_rate_half_radius` | 60 | 30 – 150 | m | Middle rate-LOD ring (must be ≥ `peer_rate_full_radius`).  Planes between `_full_radius` and `_half_radius` get **half-rate** updates (every 2nd broadcast tick).  Client-side interpolation smooths the gap — at this distance the visible motion per frame is small, so half-rate looks identical |
| `net.peer_rate_outer_factor` | 4 | 2 – 8 | (every N ticks) | Outer rate-LOD ring (between `_half_radius` and `peer_sight_range`): planes at this distance get a pose-frame every Nth broadcast tick.  Default 4 = quarter-rate (7.5 Hz at base `broadcast_rate=30`).  Visible motion is small at sight-range edge, interpolation hides the gap |
| `net.peer_interp_buffer_frames` | 3 | 2 – 8 | (count) | Per-peer pose-history buffer on the phone for interpolation.  Phone keeps the last N received pose-frames and interpolates linearly between them at render time; older frames are evicted.  Larger = smoother but more frame-delay; smaller = snappier but vulnerable to dropped frames |
| `net.bounce_lookahead_ms` | 100 | 30 – 300 | ms | How far ahead the server forecasts geometry bounces.  When a plane's ballistic projection (current pos + current velocity × t) intersects geometry within this window, the server emits a FORECAST message containing the predicted (t_bounce, pos, vel_after).  Longer lookahead = clients have more time for smooth animation prep, but more invalidations from player-input changes.  Shorter = more responsive to inputs, less animation prep time.  For plane-on-plane bounces, lookahead clamps to one broadcast tick (~33 ms) since the prediction depends on TWO trajectories with TWO sets of inputs |
| `net.forecast_correction_window_ms` | 200 | 50 – 500 | ms | How quickly the phone reconciles its locally-animated forecast against the next regular POSE update after `t_bounce`.  Shorter = faster correction but more visible "popping" when forecast was off; longer = smoother visual blend but lingering displacement.  Linear or smoothstep blend over the window |

## Phone-side smooth rendering (v1)

Client-side smoothness behaviours that hide the rate-LOD and
sight-boundary mechanics from the player.  Not QoS — these run
unconditionally in v1, same code path regardless of adaptive
scaling state.

| Parameter | Default | Range | Unit | Why |
|---|---|---|---|---|
| `view.peer_fade_in_secs` | 0.4 | 0.1 – 1.5 | s | Duration of the alpha-ramp 0 → 1 when a peer first enters sight range.  Short = peer appears decisively; long = soft introduction.  0.4 sec is roughly the time a player's eye takes to focus on a new visual element |
| `view.peer_fade_out_secs` | 0.5 | 0.1 – 1.5 | s | Duration of the alpha-ramp 1 → 0 when a peer's pose-frame stream goes silent (sight-range exit).  Slightly longer than fade-in because exits are usually less salient than entries and a slower fade is less likely to be jarring |
| `view.peer_silence_timeout_secs` | 0.25 | 0.15 – 1.0 | s | Backstop time-since-last-frame before falling back to silence-based fade-out (primary detector is the explicit `EXIT:plane_id` event from the server).  Sized as a packet-loss / server-crash safety net: > 1 outer-band pose interval (~130 ms at 7.5 Hz) plus a small jitter margin.  Smaller = faster recovery on lost EXIT messages; larger = more tolerant to packet jitter.  Previously 0.4 sec when this was the primary exit detector; lowered now that EXIT carries the load |
| `view.interp_method` | "linear" | linear / hermite | enum | Pose-frame interpolation method.  **Linear** is the default — [`tools/audience-demo-50/interp_test.loft`](../../../../tools/audience-demo-50/interp_test.loft) shows linear + bounce-keyframe is **exact** (0 mm error) for piecewise-linear motion at every sample rate, while Hermite still has 167 mm error on a kf'd bounce (the cubic curves through the corner instead of meeting it).  Hermite IS perfect on smooth curves (constant turns), but bumper-airplanes' primary motion is bounces and straight flight — sharp corners are far more common than smooth orbits.  Linear's 67 mm circular-motion error at 7.5 Hz translates to ~1 pixel at outer-band viewing distance, imperceptible.  "hermite" retained as an opt-in for scenes dominated by smooth motion |

## Adaptive QoS (post-v1 — phase 7)

Static `net.peer_*` defaults from § Networking ship in v1.  Phase 7
classifies each phone into a quality tier passively and scales its
peer-rendering envelope.  These parameters are inert in v1 (no
classification runs, scaling stays at 1.0×); they activate when
phase 7 lands.

| Parameter | Default | Range | Unit | Why |
|---|---|---|---|---|
| `qos.classify_window_secs` | 3.0 | 1.0 – 10.0 | s | Sliding window over which RTT + loss are sampled.  Shorter = more reactive but jitter-sensitive; longer = stable tier but slow to react when a connection actually changes |
| `qos.tier_upgrade_hold_secs` | 5.0 | 2.0 – 15.0 | s | A phone must stay in upgrade-conditions for this long before tier-up fires.  Hysteresis: avoids flapping between OK and Good on a connection that's marginal between them |
| `qos.tier_downgrade_hold_secs` | 1.0 | 0.5 – 5.0 | s | Faster than upgrade-hold.  Dropping a tier is protective; better to over-react and stabilise than under-react and let the player stutter |
| `qos.good_rtt_max_ms` | 50 | 20 – 100 | ms | RTT below this (with low loss) classifies as Good |
| `qos.good_loss_max_pct` | 1.0 | 0.1 – 3.0 | % | Loss-rate below this (with low RTT) classifies as Good |
| `qos.limited_rtt_min_ms` | 150 | 80 – 400 | ms | RTT above this classifies as Limited regardless of loss |
| `qos.limited_loss_min_pct` | 5.0 | 2.0 – 15.0 | % | Loss-rate above this classifies as Limited regardless of RTT |
| `qos.good_sight_scale` | 1.5 | 1.0 – 2.5 | × | Multiplier on `net.peer_sight_range` for Good-tier phones.  Good connections see further |
| `qos.limited_sight_scale` | 0.6 | 0.3 – 0.9 | × | Multiplier on `net.peer_sight_range` for Limited-tier phones.  Smaller envelope = less bandwidth needed |
| `qos.limited_full_radius_scale` | 1.5 | 1.0 – 2.5 | × | Multiplier on `net.peer_rate_full_radius` for Limited-tier.  Bigger full-rate ring (relative to sight range) so close peers still get full updates even when the phone can't afford much |
| `qos.limited_interp_buffer_frames` | 4 | 2 – 8 | (count) | Per-peer pose-history buffer for Limited-tier phones.  Deeper than the default 3 so phone-side interpolation can ride out jitter without visible stutter |

## Round structure + spawn

| Parameter | Default | Range | Unit | Why |
|---|---|---|---|---|
| `round.duration` | 300 | 120 – 600 | s | Length of a round.  5 min is the meetup-demo sweet spot |
| `round.leaderboard_hold` | 15 | 5 – 30 | s | Time the GAME OVER + leaderboard splash holds before the next round starts |
| `spawn.invincibility` | 4 | 2 – 8 | s | Post-spawn window with no collisions / no scoring.  Prevents spawn-camp grief |
| `spawn.altitude` | 30 | 15 – 80 | m | Spawn altitude relative to ground.  High enough to glide before the first input |

## World geometry (extrusion heights)

| Parameter | Default | Range | Unit | Why |
|---|---|---|---|---|
| `world.height.sea` | 0 | 0 – 0 | m | Flat |
| `world.height.grass_hill_min` | 0.5 | 0.0 – 2.0 | m | Lower bound of grass/hill extrusion |
| `world.height.grass_hill_max` | 2.0 | 1.0 – 4.0 | m | Upper bound |
| `world.height.rock_min` | 3.0 | 2.0 – 5.0 | m | Lower bound of rock/steep_rock |
| `world.height.rock_max` | 6.0 | 4.0 – 10.0 | m | Upper bound |
| `world.height.wall_min` | 8.0 | 5.0 – 12.0 | m | Pillar min height |
| `world.height.wall_max` | 12.0 | 10.0 – 18.0 | m | Pillar max height |
| `world.height.wall_high_min` | 12.0 | 10.0 – 15.0 | m | Cliff min height |
| `world.height.wall_high_max` | 20.0 | 15.0 – 30.0 | m | Cliff max height |
| `world.height.variation_hash` | 1 | 0 / 1 | flag | 0 = use min value (flat tops); 1 = vary deterministically by `hash(q, r)`.  Deterministic so the same map renders identically across reloads |

## "The storm" (optional difficulty ramp)

| Parameter | Default | Range | Unit | Why |
|---|---|---|---|---|
| `storm.enabled` | true | bool | — | Master switch — playtest may decide the storm isn't worth its complexity |
| `storm.start_time` | 180 | 60 – 240 | s | Seconds into the round when the storm begins ramping up |
| `storm.peak_time` | 270 | 200 – 300 | s | Seconds into the round when wind reaches peak |
| `storm.wind_peak_speed` | 12 | 4 – 25 | m/s | Lateral force amplitude at peak |
| `storm.fog_peak_distance` | 40 | 20 – 80 | m | Visibility radius at peak (vs unlimited at start) |

---

## How to update during playtest

1. Bisect by reachable adjacent values.  If `target.cooldown_secs = 15` feels off, try 10 first, then 20 — don't jump to 5 or 30 unless 10/20 are clearly worse.
2. **Change one parameter at a time.**  Coupled adjustments hide which knob actually moved the result.
3. After each playtest pass, record the verdict here in a `## Playtest log` section at the bottom of this file (date, parameter changed, finding).
4. Once a value has been stable for ≥ 2 playtests, lock it as the new default and update the table above.

## See also

- [`README.md`](README.md) — design narrative.  Inline values
  there are illustrative; this file is canonical.
- Dryopea's [`docs/NUMBERS.md`](https://github.com/jjstwerff/dryopea/blob/main/docs/NUMBERS.md)
  — pattern this file follows.
- [`README.md` § Open questions](README.md#open-questions-not-blockers-for-the-draft)
  — questions that won't be answered by a value in this file
  (they're design decisions, not tuning).
