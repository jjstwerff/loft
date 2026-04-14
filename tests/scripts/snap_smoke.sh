#!/bin/bash
# Capture a screenshot of `lib/graphics/examples/00-smoke.loft` rendering
# under Xvfb. Used by `make test-gl-golden` to compare against
# `tests/golden/00-smoke.png`.
#
# Args: $1 = output PNG path
#
# Implementation note: 00-smoke.loft has a finite render loop (200 frames)
# so it would exit before the screenshot is captured. We wrap the same
# rendering body in a long-running loop here so the window stays alive
# during capture, then take the snapshot via xdotool + import.

set -e
OUTPUT="${1:?output png path required}"

cd "$(dirname "$0")/../.."

cat > /tmp/loft_smoke_long.loft << 'LOFT_EOF'
use graphics;
SCREEN_W = 400.0;
SCREEN_H = 300.0;
fn build_atlas() -> graphics::Canvas {
  c = graphics::canvas(64, 64, 0);
  c.fill_rect(0,  0,  32, 32, graphics::rgba(220,  60,  60, 255));
  c.fill_rect(32, 0,  32, 32, graphics::rgba( 60, 220,  60, 255));
  c.fill_rect(0,  32, 32, 32, graphics::rgba( 60,  60, 220, 255));
  c.fill_rect(32, 32, 32, 32, graphics::rgba(230, 230, 230, 255));
  c
}
fn main() {
  if !graphics::gl_create_window(SCREEN_W as integer, SCREEN_H as integer, "smoke") { return; }
  painter = graphics::create_painter_2d(SCREEN_W, SCREEN_H);
  atlas = build_atlas();
  sheet = graphics::create_sprite_sheet(atlas, 2, 2, graphics::painter_vao(painter));
  banner = graphics::canvas(120, 20, graphics::rgba(40, 40, 60, 255));
  banner.fill_rect(2, 2, 116, 16, graphics::rgba(200, 200, 220, 255));
  banner_tex = graphics::gl_upload_canvas(banner.data, banner.width, banner.height);
  font = graphics::gl_load_font("DejaVuSans-Bold.ttf");
  if !font { font = graphics::gl_load_font("lib/graphics/examples/DejaVuSans-Bold.ttf"); }
  text_tex = 0;
  if font {
    text_tex = graphics::create_text_texture(font, "SCORE", 18.0, graphics::rgba(255, 255, 255, 255));
  }
  for _f in 0..100000 {
    if !graphics::gl_poll_events() { break; }
    graphics::gl_clear(graphics::rgba(20, 25, 35, 255));
    for sm_i in 0..6 {
      sm_x = 10.0 + sm_i as float * 60.0;
      sm_a = 0.4 + sm_i as float * 0.1;
      graphics::draw_rect_at(painter, sm_x, 20.0, 50.0, 30.0,
        sm_i as float * 0.15, 0.5, 1.0 - sm_i as float * 0.15, sm_a);
    }
    for sm_si in 0..4 {
      sm_sx = 10.0 + sm_si as float * 60.0;
      graphics::draw_sprite_at(sheet, painter, sm_sx, 70.0, 32.0, 32.0, sm_si);
    }
    graphics::draw_texture_at(painter, banner_tex, 10.0, 120.0, 120.0, 20.0);
    if text_tex {
      graphics::draw_texture_at(painter, text_tex, 200.0, 120.0, 80.0, 20.0);
    }
    graphics::draw_rect_at(painter, 0.0, SCREEN_H - 4.0, SCREEN_W, 4.0, 0.3, 0.6, 1.0, 1.0);
    graphics::gl_swap_buffers();
  }
}
LOFT_EOF

./target/release/loft --interpret \
  --path "$(pwd)/" --lib "$(pwd)/lib/" \
  /tmp/loft_smoke_long.loft >/tmp/loft_smoke.log 2>&1 &
LOFT_PID=$!

# Poll for window to appear (max 5s)
WIN_ID=""
for i in 1 2 3 4 5 6 7 8 9 10; do
  sleep 0.5
  WIN_ID=$(xdotool search --name "." 2>/dev/null | tail -1)
  [ -n "$WIN_ID" ] && break
done

# Let it render a few frames
sleep 1

if [ -z "$WIN_ID" ]; then
  echo "FAIL: no loft window"
  cat /tmp/loft_smoke.log
  kill $LOFT_PID 2>/dev/null
  exit 1
fi

import -window "$WIN_ID" "$OUTPUT" 2>/tmp/loft_import.log
RC=$?

# P133: Xvfb/Mesa-swrast framebuffer + ImageMagick `import` reads pixels with
# R and B channels swapped.  gl_clear(rgba(20, 25, 35, 255)) renders
# correctly on-screen but the captured PNG stores (35, 25, 20).  Swap the
# channels back so the golden PNG has the colours the user actually asked
# for.  On a real display this step would be a no-op; it is a pure
# transformation of the Xvfb-captured buffer.
if [ $RC -eq 0 ]; then
  convert "$OUTPUT" -separate -swap 0,2 -combine "$OUTPUT" 2>>/tmp/loft_import.log
  RC=$?
fi

kill $LOFT_PID 2>/dev/null || true
wait $LOFT_PID 2>/dev/null || true
rm -f /tmp/loft_smoke_long.loft

if [ $RC -ne 0 ]; then
  echo "FAIL: import failed"
  cat /tmp/loft_import.log
  exit $RC
fi

# C58 regression guard: the four sprites drawn at y=70 come from a 2×2
# atlas whose quadrants are (in canvas coords) red, green, blue, white.
# If the canvas→GL Y-flip cascade ever reintroduces a row swap, sprites
# 0/2 and 1/3 will exchange colours and this check fails.
probe() {
  convert "$OUTPUT" -format "%[pixel:p{$1,$2}]" info:
}
want() {
  got=$(probe "$1" "$2")
  if [ "$got" != "$3" ]; then
    echo "FAIL: sprite row-mapping regressed at ($1, $2) — got $got, want $3"
    echo "      If the canvas Y convention changed intentionally, update this assert."
    exit 1
  fi
}
want 26  86 "srgb(220,60,60)"    # sprite 0 = atlas row 0 col 0 = red
want 86  86 "srgb(60,220,60)"    # sprite 1 = atlas row 0 col 1 = green
want 146 86 "srgb(60,60,220)"    # sprite 2 = atlas row 1 col 0 = blue
want 206 86 "srgb(230,230,230)"  # sprite 3 = atlas row 1 col 1 = white

exit 0
