# Debugging and testing interactive loft games

This document explains how to run, inspect, and regression-test
interactive GL programs (games, editors, demos) **headlessly** — without
a monitor, without a window manager, without repeatedly mashing keys by
hand.  The workflow is battle-tested on
`lib/graphics/examples/25-brick-buster.loft` and designed to generalise to
every long-running GL example.

Tools used: `xvfb-run` (headless X server), `xdotool` (X automation),
ImageMagick `import` / `compare` (screenshot capture + pixel diff).

## Quick overview — the pieces

| File | Purpose |
|---|---|
| `tests/scripts/snap_example.sh` | Launches any long-running `.loft` example, finds its window, optionally sends scripted keys, captures a screenshot, swaps R↔B (Xvfb artifact). |
| `tests/scripts/gl_snapshots.tsv` | Test registry: one line per snapshot test (example, golden, wait, window regex, geometry, key script). |
| `tests/scripts/test_gl_snapshots.sh` | Runs every row in the registry under Xvfb and compares with `tests/golden/` or regenerates the goldens with `--update`. |
| `tests/golden/*.png` | Reference images that committed snapshots must match pixel-for-pixel (1% fuzz). |

## Running the validation suite

```bash
# Compare every captured snapshot against its golden.
tests/scripts/test_gl_snapshots.sh

# Regenerate every golden after an intentional visual change.
tests/scripts/test_gl_snapshots.sh --update
```

Both targets are safe to run from a CI job: they skip cleanly if
`xvfb-run`, `xdotool`, `import`, `compare`, or `convert` are missing.

## Adding a new snapshot test

1. Pick a long-running example (a finite 200-frame demo exits before
   the capture — wrap its body in a 1_000_000-iteration loop first, or
   use a dedicated helper à la `tests/scripts/snap_smoke.sh`).
2. Add a row to `tests/scripts/gl_snapshots.tsv` with tab-separated
   fields:
   ```
   <example.loft>    <golden.png>   <wait_s>   <window_regex>   <geometry>   [<key_script>]
   ```
   - `window_regex` is a case-insensitive `xdotool --name` pattern;
     use a substring of the window title set via
     `graphics::gl_create_window(w, h, "Title")`.
   - `geometry` is the Xvfb display (`WxHxDEPTH`, e.g. `800x600x24`).
   - `key_script` (optional) is a `;`-separated list of `KEY@MS`
     pairs — see next section.
3. `tests/scripts/test_gl_snapshots.sh --update` creates/overwrites the
   goldens.
4. Inspect `tests/golden/<name>.png` visually; if it looks right,
   commit the registry row **and** the golden.

## Driving a game to a specific state — key scripts

The `key_script` column feeds xdotool before capture.  Each step sends
one key, then sleeps.  Example from `gl_snapshots.tsv`:

```
25-brick-buster.loft   25-brick-buster-paused.png    1   rick Buster   800x600x24   space@500;p@300
```

reads as: *"launch Brick Buster, wait 1 s for the window, press SPACE,
wait 500 ms (the game has ~30 frames to react), press P, wait 300 ms,
capture."*

Key names are whatever `xdotool key NAME` accepts:
- letters: `a`, `b`, `k`, `space`
- specials: `Return`, `Tab`, `Escape`, `Left`, `Right`, `Up`, `Down`
- function keys: `F1`, `F2`, ...

### Cheat keys for hard-to-reach states

Natural input can take many seconds to drive a game into an interesting
state (waiting 10 s for the ball to fall off the paddle three times to
reach game-over is both slow and flaky).  Add **cheat keys** to the
game that are harmless during normal play but jump straight to a target
state when pressed.  See `lib/graphics/examples/25-brick-buster.loft`:

```loft
// Test cheat keys — not bound to any normal gameplay action.
//   K (107)  — force game over from playing state
//   X (120)  — trigger paddle-explosion phase
//   U (117)  — spawn all 8 pickups falling from mid-screen
br_cheat_k_down = graphics::gl_key_pressed(107);
br_cheat_k_pressed = br_cheat_k_down && !br_cheat_k_was_down;
br_cheat_k_was_down = br_cheat_k_down;
// ...

if br_game_state == GS_PLAYING {
  if br_cheat_k_pressed { br_lost = true; }
  if br_cheat_x_pressed && br_paddle_state == 0 {
    for br_i in 0..3 { br_ball_on[br_i] = 0; }  // force "all balls lost" branch
  }
  if br_cheat_u_pressed {
    for br_i in 0..8 {
      br_pk_x[br_i] = (br_i as float + 0.5) * (SCREEN_W / 8.0) - PICKUP_W / 2.0;
      br_pk_y[br_i] = 100.0 + (br_i as float) * 12.0;
      br_pk_type[br_i] = 1 + (br_i % 7);
      br_pk_on[br_i] = 1;
    }
  }
}
```

Then register one row per state:

```
25-brick-buster.loft   25-brick-buster-gameover.png    1   rick Buster   800x600x24   space@500;k@400
25-brick-buster.loft   25-brick-buster-explosion.png   1   rick Buster   800x600x24   space@500;x@600
25-brick-buster.loft   25-brick-buster-powerups.png    1   rick Buster   800x600x24   space@500;u@400
```

Guidelines for cheat keys:
- **Edge-detected** (`pressed = down && !was_down`) so holding the
  cheat key does not re-trigger every frame.
- **Pick keys that are not gameplay-bound** (A/D for movement, P for
  pause — so use K/U/X instead).
- **Keep them minimal** — one cheat per hard-to-reach state.
- **Leave them in** even for release: the overhead is negligible and
  being able to jump to a state speeds up human debugging too.

### Getting key events through Xvfb

Xvfb has no window manager, so `xdotool windowactivate` fails with
`Your windowmanager claims not to support _NET_ACTIVE_WINDOW`.
`windowfocus` alone is sufficient — `snap_example.sh` focuses the
loft window once and then sends each key with plain `xdotool key`.
`xdotool key` holds the press long enough (~20–30 ms, spanning 1–2
frames at 60 fps) for edge-detected handlers in the game to latch the
transition.

If a particular key fails to register:
1. Run the example by hand and verify the key works in normal play.
2. Check that the cheat code runs inside the expected state branch
   (gated on `br_game_state == GS_PLAYING` etc.).
3. Add a temporary `println("DEBUG key seen, state={s}")` inside the
   handler — but beware stdout buffering (see below).

## Inspecting a live headless run

When a state isn't captured correctly and you need to see what the game
is actually doing, run it directly under Xvfb and send keys manually:

```bash
xvfb-run -a -s "-screen 0 800x600x24" bash -c '
  ./target/release/loft --interpret --path "$(pwd)/" --lib "$(pwd)/lib/" \
    lib/graphics/examples/25-brick-buster.loft > /tmp/loft.log 2>&1 &
  pid=$!
  sleep 1.5                       # wait for window
  WID=$(xdotool search --name rick Buster | tail -1)
  xdotool windowfocus "$WID"
  xdotool key space               # start game
  sleep 0.5
  xdotool key u                   # spawn pickups (cheat)
  sleep 0.5
  import -window "$WID" /tmp/frame.png
  convert /tmp/frame.png -separate -swap 0,2 -combine /tmp/frame.png
  kill $pid; wait $pid 2>/dev/null
'
cat /tmp/loft.log
# Inspect /tmp/frame.png in your editor or with `xdg-open`.
```

### Stdout buffering gotcha

Rust `println!` is **block-buffered** when stdout is redirected to a
file or pipe, **not** line-buffered like a terminal.  A `println!` in
your game may never reach the log file if the process is killed
before the 8 KB buffer fills.

Workarounds:
- Write debug output with `eprintln!`-style stderr (loft has no
  equivalent today — file an issue if you need one).
- Let the game exit naturally by making it observable via screenshot
  differences rather than stdout prints.  Visual state is always
  flushed through the framebuffer on every `gl_swap_buffers` call.
- For deep debugging, wrap `println!` with a flush call in the
  stdlib (see `default/03_text.loft`).

### The Xvfb R↔B channel swap (P133)

Under Xvfb + Mesa-swrast, ImageMagick `import` reads the framebuffer
with **R and B swapped**.  `gl_clear(rgba(20, 25, 35, 255))` renders
correctly on-screen but the raw capture stores `(35, 25, 20)`.

`snap_example.sh` and `tests/scripts/snap_smoke.sh` post-process the
capture with:

```bash
convert <png> -separate -swap 0,2 -combine <png>
```

On a real display this step is a no-op; under Xvfb it restores the
colours the loft program actually asked for.  Goldens must be
captured with the swap applied so that they hold the true colours.

## Verifying the existing Brick Buster suite

```bash
tests/scripts/test_gl_snapshots.sh
#   25-brick-buster-title.png          ok (0 px differ)
#   25-brick-buster-playing.png        ok (0 px differ)
#   25-brick-buster-paused.png         ok (0 px differ)
#   25-brick-buster-powerups.png       ok (0 px differ)
#   25-brick-buster-explosion.png      ok (0 px differ)
#   25-brick-buster-gameover.png       ok (0 px differ)
```

Each golden covers a distinct `GS_*` state or visual effect.  A future
commit that accidentally breaks the pickup renderer, the paddle
explosion animation, or the pause overlay fails this test immediately
with a per-pixel diff written to
`/tmp/loft_gl_snapshots/<name>.png.diff.png` for inspection.

## See also

- [TESTING.md](TESTING.md) § Using Xvfb — the older single-example
  pipeline (`snap_smoke.sh` + `make test-gl-golden`) that the new
  `test_gl_snapshots.sh` generalises.
- [PROBLEMS.md](PROBLEMS.md) P133 — background on the R↔B swap and why
  the post-process `-swap 0,2` is needed.
- `lib/graphics/examples/25-brick-buster.loft` — reference implementation
  of edge-detected cheat keys (`br_cheat_k_pressed`, etc.).
