
# OpenGL / WebGL / GLB Library Design

Design for a loft graphics library covering 2D RGBA drawing, 3D mesh representation,
scene management, and multi-backend rendering (desktop OpenGL, browser WebGL, GLB file export).

---

## Contents

- [Philosophy](#philosophy)
- [File layout](#file-layout)
- [Native ops — minimal set](#native-ops--minimal-set)
- [2D Drawing — types](#2d-drawing--types)
- [2D Drawing — canvas](#2d-drawing--canvas)
- [2D Drawing — rasterization in loft](#2d-drawing--rasterization-in-loft)
- [2D Drawing — primitives in loft](#2d-drawing--primitives-in-loft)
- [2D Text rendering](#2d-text-rendering)
- [3D Math — loft implementations](#3d-math--loft-implementations)
- [3D Mesh — types](#3d-mesh--types)
- [3D Scene — types](#3d-scene--types)
- [Material and texture](#material-and-texture)
- [GLB export — loft implementation](#glb-export--loft-implementation)
- [OpenGL backend](#opengl-backend)
- [WebGL backend](#webgl-backend)
- [Rust integration notes](#rust-integration-notes)
- [Usage examples](#usage-examples)
- [Implementation constraints](#implementation-constraints)

---

## Philosophy

### Real-world optimization target

This library is also a **benchmark suite for the loft interpreter**.  The graphics
workloads — iterating over millions of pixels, tight Bezier subdivision loops, scanline
fill over large canvases, matrix math per frame — are exactly the kind of real-world,
compute-intensive scenarios that expose interpreter bottlenecks that microbenchmarks
miss.

The deliberate choice to implement the rasterizer, matrix math, and GLB writer in loft
(rather than hiding them in Rust) makes this a continuous performance contract:

> *If `wu_line` on a 1024×1024 canvas is acceptably fast in loft, the interpreter is
> ready for production compute workloads.  If it is not, the bottleneck is visible and
> measurable, and the fix goes into the interpreter.*

Each phase of [OPENGL_IMPL.md](OPENGL_IMPL.md) therefore doubles as a performance
regression test.  Benchmark numbers should be recorded in [PERFORMANCE.md](PERFORMANCE.md)
as each phase is completed.

---

### Implementation split

Almost all library logic is implemented in loft.  Rust provides only operations that
loft structurally cannot express:

| Category | In Rust | In loft |
|---|---|---|
| PNG encode/decode | yes (binary codec) | — |
| Font parsing + glyph rasterization | yes (fontdue) | — |
| IEEE 754 float bit pattern | yes (`single_bits`) | — |
| Raw binary file write | yes | — |
| GPU API calls (GL / WebGL) | yes (FFI) | — |
| Window / event loop | yes (GLFW / web-sys) | — |
| All canvas pixel operations | — | yes |
| Anti-aliased line rasterizer | — | yes |
| Bezier subdivision | — | yes |
| Scanline fill | — | yes |
| All 2D primitives | — | yes |
| Glyph layout, alignment, wrapping | — | yes |
| Glyph bitmap compositing | — | yes |
| 4×4 matrix math | — | yes |
| Look-at / perspective / TRS matrices | — | yes |
| Euler → quaternion | — | yes |
| GLB JSON chunk building | — | yes |
| GLB BIN buffer assembly | — | yes |
| Scene traversal + draw call setup | — | yes |

---

## File layout

Field lookups are type-scoped, so field name overlap across structs is safe.
The library uses separate files for modularity:

```
lib/graphics/
  draw.loft      — Rgba, Xy, PathNode, Draw, LineStyle, Canvas + rasterizer
  primitives.loft — rect, ellipse, rounded_rect, arrow (produce Draw values)
  text.loft      — Font, GlyphMetrics, GlyphBitmap, TextStyle, draw_text*
  math.loft      — Mat4 (vector<single>), all matrix and vector math operations
  mesh.loft      — Vec2, Vec3, Vec4, Vertex, Triangle, Mesh
  scene.loft     — Transform, MeshInstance, Camera, Light, Scene
  texture.loft   — TextureSource, Material
  glb.loft       — GLB binary writer (JSON + BIN in loft, file write via native)
  opengl.loft    — thin OpGL* native ops + loft render/upload functions
  webgl.loft     — thin OpWgl* native ops + loft equivalents
```

---

## Native ops — minimal set

These are the only operations backed by `#rust` annotations.  Everything else is loft.

### PNG codec

```loft
// Encode canvas pixels as PNG and write to path.
fn OpSavePng(canvas: Canvas, path: text);
#rust"stores.write_png(@path, &@canvas);"

// Decode a PNG file into a new Canvas (canvas_size = max(width,height)).
// Returns a zero-size Canvas on error.
fn OpLoadPng(path: text) -> Canvas;
#rust"stores.load_png(@path)"
```

### Font / glyph

```loft
// Parse a TTF/OTF file and store in the interpreter font registry.
// Returns the assigned font_id (>= 0), or -1 on failure.
fn OpLoadFont(path: text) -> integer;
#rust"drawing::load_font(@path)"

// Return the font_id of the bundled fallback font (always succeeds).
fn OpDefaultFont() -> integer;
#rust"drawing::default_font()"

// Per-character layout metrics for font_id at size pixels.
pub struct GlyphMetrics {
  gm_advance: single not null,  // horizontal advance in pixels
  gm_ascent:  single not null,  // distance above baseline
  gm_descent: single not null,  // distance below baseline (positive)
  gm_left:    integer not null, // left bearing in pixels
  gm_top:     integer not null, // top bearing in pixels (below baseline origin)
  gm_width:   integer not null, // bitmap width
  gm_height:  integer not null  // bitmap height
}

// 8-bit coverage bitmap produced by fontdue.
pub struct GlyphBitmap {
  gb_width:  integer not null,
  gb_height: integer not null,
  gb_pixels: vector<u8>         // row-major, 0 = transparent, 255 = opaque
}

// Return layout metrics for a single character; does not rasterize.
fn OpGlyphMetrics(font_id: integer, ch: character, size: single) -> GlyphMetrics;
#rust"drawing::glyph_metrics(@font_id, @ch, @size)"

// Rasterize a single character; returns coverage bitmap.
fn OpRasterizeGlyph(font_id: integer, ch: character, size: single) -> GlyphBitmap;
#rust"drawing::rasterize_glyph(@font_id, @ch, @size)"
```

### Binary encoding

```loft
// Return the IEEE 754 bit pattern of v as a signed integer.
// Used by the GLB binary buffer writer in loft.
fn OpSingleBits(v: single) -> integer;
#rust"(@v).to_bits() as i32"

// Write raw bytes to path (overwrites existing file).
fn OpWriteBytes(path: text, data: vector<u8>) -> boolean;
#rust"stores.write_bytes(@path, &@data)"
```

### OpenGL / WebGL GPU calls

Thin wrappers for individual GPU API calls — see [OpenGL backend](#opengl-backend) and
[WebGL backend](#webgl-backend).  Every piece of logic (which objects to draw, matrices,
uniforms) is computed in loft before calling these.

---

## 2D Drawing — types

### `draw.loft`

```loft
// 32-bit RGBA colour. a == 0 means fully transparent / absent.
pub struct Rgba {
  r: u8 not null,
  g: u8 not null,
  b: u8 not null,
  a: u8 not null
}

// 2D coordinate in canvas pixels (origin top-left, y increases downward).
pub struct Xy {
  x: single not null,
  y: single not null
}

// A node in a vector path.
// Point — simple anchor; stroke passes through coord.
//         a == 0 in color means "move without drawing" at this node.
// Curve — cubic-Bezier anchor; in_handle and out_handle are offsets
//         relative to coord.  The curve between two adjacent Curve nodes
//         is the cubic Bezier:
//           prev.coord, prev.coord + prev.out_handle,
//           this.coord + this.in_handle, this.coord
pub enum PathNode {
  Point { coord: Xy, color: Rgba, size: single },
  Curve { coord: Xy, color: Rgba, size: single, in_handle: Xy, out_handle: Xy }
}

// Stroke style applied between anchor nodes.
pub enum LineStyle { Straight, Dashed, Dotted }

// A complete vector path.
// fill.a == 0 = no fill.  Use LineStyle to control gap pattern.
pub struct Draw {
  nodes: vector<PathNode>,
  fill:  Rgba,
  line:  LineStyle
}
```

---

## 2D Drawing — canvas

```loft
// An RGBA pixel buffer of canvas_size × canvas_size pixels (square).
// All pixels initialise to Rgba{r:0,g:0,b:0,a:0} (fully transparent).
pub struct Canvas {
  canvas_size: integer not null,
  canvas_data: vector<Rgba>
}

pub fn canvas(size: integer) -> Canvas {
  c = Canvas { canvas_size: size };
  blank = Rgba { r: 0, g: 0, b: 0, a: 0 };
  for _ci in 0..size * size {
    c.canvas_data += [blank];
  }
  c
}

pub fn pixel_at(self: Canvas, x: integer, y: integer) -> Rgba {
  if x < 0 || y < 0 || x >= self.canvas_size || y >= self.canvas_size {
    return Rgba { r: 0, g: 0, b: 0, a: 0 };
  }
  self.canvas_data[y * self.canvas_size + x]
}

pub fn set_pixel(self: &Canvas, x: integer, y: integer, color: Rgba) {
  if x >= 0 && y >= 0 && x < self.canvas_size && y < self.canvas_size {
    self.canvas_data[y * self.canvas_size + x] = color;
  }
}

// Porter-Duff src-over composite.
pub fn blend_pixel(self: &Canvas, x: integer, y: integer, src: Rgba) {
  if x < 0 || y < 0 || x >= self.canvas_size || y >= self.canvas_size { return }
  bp_idx = y * self.canvas_size + x;
  bp_dst = self.canvas_data[bp_idx];
  bp_sa  = src.a;
  bp_da  = bp_dst.a;
  bp_out = bp_sa + bp_da * (255 - bp_sa) / 255;
  if bp_out == 0 {
    self.canvas_data[bp_idx] = Rgba { r: 0, g: 0, b: 0, a: 0 };
    return
  }
  self.canvas_data[bp_idx] = Rgba {
    r: (src.r * bp_sa + bp_dst.r * bp_da * (255 - bp_sa) / 255) / bp_out,
    g: (src.g * bp_sa + bp_dst.g * bp_da * (255 - bp_sa) / 255) / bp_out,
    b: (src.b * bp_sa + bp_dst.b * bp_da * (255 - bp_sa) / 255) / bp_out,
    a: bp_out
  };
}

// Blend with an explicit fractional alpha weight in [0.0, 1.0].
// Used by the anti-aliased rasterizer.
fn blend_alpha(self: &Canvas, x: integer, y: integer, color: Rgba, alpha: single) {
  if alpha <= 0.0f { return }
  a8 = round(alpha * 255.0f) as integer;
  self.blend_pixel(x, y, Rgba { r: color.r, g: color.g, b: color.b, a: a8 });
}

pub fn save_png(self: Canvas, path: text) {
  OpSavePng(self, path);
}

pub fn load_png(path: text) -> Canvas {
  OpLoadPng(path)
}
```

---

## 2D Drawing — rasterization in loft

All rasterization is pure loft, calling `blend_pixel` / `blend_alpha` from above.

### Anti-aliased line (Xiaolin Wu)

```loft
// Fractional part of x.
fn wu_fpart(x: single) -> single { x - floor(x) }
fn wu_rfpart(x: single) -> single { 1.0f - wu_fpart(x) }

// Draw one anti-aliased line segment from (x0,y0) to (x1,y1).
fn wu_line(self: &Canvas, x0: single, y0: single,
           x1: single, y1: single, color: Rgba) {
  wu_dx   = x1 - x0;
  wu_dy   = y1 - y0;
  wu_steep = abs(wu_dy) > abs(wu_dx);
  if wu_steep {
    wu_tmp = x0; x0 = y0; y0 = wu_tmp;
    wu_tmp = x1; x1 = y1; y1 = wu_tmp;
  }
  if x0 > x1 {
    wu_tmp = x0; x0 = x1; x1 = wu_tmp;
    wu_tmp = y0; y0 = y1; y1 = wu_tmp;
  }
  wu_dx   = x1 - x0;
  wu_dy   = y1 - y0;
  wu_grad = if wu_dx == 0.0f { 1.0f } else { wu_dy / wu_dx };

  // first endpoint
  wu_xe  = floor(x0 + 0.5f);
  wu_ye  = y0 + wu_grad * (wu_xe - x0);
  wu_xg  = wu_rfpart(x0 + 0.5f);
  wu_px1 = wu_xe as integer;
  wu_py1 = floor(wu_ye) as integer;
  if wu_steep {
    self.blend_alpha(wu_py1,     wu_px1, color, wu_rfpart(wu_ye) * wu_xg);
    self.blend_alpha(wu_py1 + 1, wu_px1, color, wu_fpart(wu_ye)  * wu_xg);
  } else {
    self.blend_alpha(wu_px1, wu_py1,     color, wu_rfpart(wu_ye) * wu_xg);
    self.blend_alpha(wu_px1, wu_py1 + 1, color, wu_fpart(wu_ye)  * wu_xg);
  }
  wu_intery = wu_ye + wu_grad;

  // second endpoint
  wu_xe  = floor(x1 + 0.5f);
  wu_ye  = y1 + wu_grad * (wu_xe - x1);
  wu_xg  = wu_fpart(x1 + 0.5f);
  wu_px2 = wu_xe as integer;
  wu_py2 = floor(wu_ye) as integer;
  if wu_steep {
    self.blend_alpha(wu_py2,     wu_px2, color, wu_rfpart(wu_ye) * wu_xg);
    self.blend_alpha(wu_py2 + 1, wu_px2, color, wu_fpart(wu_ye)  * wu_xg);
  } else {
    self.blend_alpha(wu_px2, wu_py2,     color, wu_rfpart(wu_ye) * wu_xg);
    self.blend_alpha(wu_px2, wu_py2 + 1, color, wu_fpart(wu_ye)  * wu_xg);
  }

  // main loop — each pixel column between endpoints
  for wu_xi in (wu_px1 + 1)..(wu_px2) {
    wu_iy = floor(wu_intery) as integer;
    if wu_steep {
      self.blend_alpha(wu_iy,     wu_xi, color, wu_rfpart(wu_intery));
      self.blend_alpha(wu_iy + 1, wu_xi, color, wu_fpart(wu_intery));
    } else {
      self.blend_alpha(wu_xi, wu_iy,     color, wu_rfpart(wu_intery));
      self.blend_alpha(wu_xi, wu_iy + 1, color, wu_fpart(wu_intery));
    }
    wu_intery += wu_grad;
  }
}
```

### Bezier segment subdivision

```loft
// Adaptive cubic Bezier subdivision → list of polyline points.
// p0..p3 are the four control points; result is appended to pts.
fn bezier_subdivide(p0: Xy, p1: Xy, p2: Xy, p3: Xy,
                    pts: &vector<Xy>, tolerance: single) {
  // Flatness: distance from midpoint of chord to midpoint of curve.
  bz_mx = (p0.x + p3.x) / 2.0f;
  bz_my = (p0.y + p3.y) / 2.0f;
  bz_cx = (p0.x + 3.0f * p1.x + 3.0f * p2.x + p3.x) / 8.0f;
  bz_cy = (p0.y + 3.0f * p1.y + 3.0f * p2.y + p3.y) / 8.0f;
  bz_flat = abs(bz_cx - bz_mx) + abs(bz_cy - bz_my);
  if bz_flat <= tolerance {
    pts += [p3];
    return
  }
  // De Casteljau split at t = 0.5
  bz_p01 = Xy { x: (p0.x + p1.x) / 2.0f, y: (p0.y + p1.y) / 2.0f };
  bz_p12 = Xy { x: (p1.x + p2.x) / 2.0f, y: (p1.y + p2.y) / 2.0f };
  bz_p23 = Xy { x: (p2.x + p3.x) / 2.0f, y: (p2.y + p3.y) / 2.0f };
  bz_p012 = Xy { x: (bz_p01.x + bz_p12.x) / 2.0f, y: (bz_p01.y + bz_p12.y) / 2.0f };
  bz_p123 = Xy { x: (bz_p12.x + bz_p23.x) / 2.0f, y: (bz_p12.y + bz_p23.y) / 2.0f };
  bz_mid  = Xy { x: (bz_p012.x + bz_p123.x) / 2.0f, y: (bz_p012.y + bz_p123.y) / 2.0f };
  bezier_subdivide(p0, bz_p01, bz_p012, bz_mid,  pts, tolerance);
  bezier_subdivide(bz_mid, bz_p123, bz_p23, p3,  pts, tolerance);
}
```

### Scanline fill

```loft
// Fill the interior of a closed polyline (list of Xy) with color.
// Uses an odd-even scanline algorithm with per-row edge sorting.
fn scanline_fill(self: &Canvas, pts: vector<Xy>, color: Rgba) {
  if len(pts) < 3 { return }
  // Bounding box
  sf_miny = pts[0].y;
  sf_maxy = pts[0].y;
  for sf_p in pts {
    if sf_p.y < sf_miny { sf_miny = sf_p.y }
    if sf_p.y > sf_maxy { sf_maxy = sf_p.y }
  }
  sf_y0 = floor(sf_miny) as integer;
  sf_y1 = floor(sf_maxy) as integer;
  sf_n  = len(pts);
  for sf_row in sf_y0..=sf_y1 {
    sf_fy   = sf_row as single + 0.5f;
    sf_xs: vector<single> = [];
    for sf_ei in 0..sf_n {
      sf_aj = pts[sf_ei];
      sf_bj = pts[(sf_ei + 1) % sf_n];
      if (sf_aj.y <= sf_fy && sf_bj.y > sf_fy) ||
         (sf_bj.y <= sf_fy && sf_aj.y > sf_fy) {
        sf_t  = (sf_fy - sf_aj.y) / (sf_bj.y - sf_aj.y);
        sf_xs += [sf_aj.x + sf_t * (sf_bj.x - sf_aj.x)];
      }
    }
    sort(sf_xs);
    sf_xi = 0;
    while sf_xi + 1 < len(sf_xs) {
      sf_x0 = floor(sf_xs[sf_xi])     as integer;
      sf_x1 = floor(sf_xs[sf_xi + 1]) as integer;
      for sf_px in sf_x0..=sf_x1 {
        self.set_pixel(sf_px, sf_row, color);
      }
      sf_xi += 2;
    }
  }
}
```

### Stroke dash pattern

```loft
// Return true if position t (cumulative path length) should be drawn
// under the given LineStyle.
fn stroke_visible(style: LineStyle, t: single) -> boolean {
  match style {
    LineStyle.Straight => true,
    LineStyle.Dashed   => (t % 12.0f) < 8.0f,
    LineStyle.Dotted   => (t % 6.0f)  < 2.0f,
  }
}
```

### Main `draw` function

```loft
// Tessellate a Draw path and rasterize it onto the canvas.
pub fn draw(self: &Canvas, path: Draw) {
  if len(path.nodes) == 0 { return }

  // --- tessellate nodes into a flat polyline ---
  dr_pts: vector<Xy> = [];
  dr_n = len(path.nodes);
  for dr_i in 0..dr_n {
    dr_cur = path.nodes[dr_i];
    dr_coord = match dr_cur {
      PathNode.Point { coord } => coord,
      PathNode.Curve { coord } => coord,
    };
    if dr_i == 0 {
      dr_pts += [dr_coord];
    } else {
      dr_prev = path.nodes[dr_i - 1];
      dr_prev_coord = match dr_prev {
        PathNode.Point { coord } => coord,
        PathNode.Curve { coord } => coord,
      };
      match dr_cur {
        PathNode.Curve { coord, in_handle } =>
          match dr_prev {
            PathNode.Curve { coord, out_handle } => {
              dr_c1 = Xy { x: dr_prev_coord.x + out_handle.x,
                           y: dr_prev_coord.y + out_handle.y };
              dr_c2 = Xy { x: dr_coord.x + in_handle.x,
                           y: dr_coord.y + in_handle.y };
              bezier_subdivide(dr_prev_coord, dr_c1, dr_c2, dr_coord,
                               dr_pts, 0.5f);
            },
            _ => { dr_pts += [dr_coord]; }
          },
        _ => { dr_pts += [dr_coord]; }
      };
    }
  }

  // --- fill interior ---
  if path.fill.a > 0 {
    scanline_fill(self, dr_pts, path.fill);
  }

  // --- stroke ---
  dr_np = len(dr_pts);
  dr_seg_i = 0;
  while dr_seg_i + 1 < dr_np {
    dr_a   = dr_pts[dr_seg_i];
    dr_b   = dr_pts[dr_seg_i + 1];
    // colour at this segment: interpolate from source nodes (simplified: use start node)
    dr_src = path.nodes[0];
    dr_col = match dr_src {
      PathNode.Point { color } => color,
      PathNode.Curve { color } => color,
    };
    dr_sz = match dr_src {
      PathNode.Point { size } => size,
      PathNode.Curve { size } => size,
    };
    if dr_col.a > 0 && stroke_visible(path.line, dr_seg_i as single) {
      // For size > 1, draw parallel offsets (simple thick line via multiple passes)
      dr_half = (dr_sz / 2.0f) as integer;
      for dr_off in 0..=dr_half {
        wu_line(self, dr_a.x, dr_a.y + dr_off as single,
                      dr_b.x, dr_b.y + dr_off as single, dr_col);
        if dr_off > 0 {
          wu_line(self, dr_a.x, dr_a.y - dr_off as single,
                        dr_b.x, dr_b.y - dr_off as single, dr_col);
        }
      }
    }
    dr_seg_i += 1;
  }
}
```

---

## 2D Drawing — primitives in loft

All primitives produce `Draw` values; render with `canvas.draw(path)`.

### `primitives.loft`

```loft
use lib/graphics/draw;

pub fn rect(x: single, y: single, w: single, h: single,
            fill: Rgba, stroke: Rgba, stroke_size: single,
            style: LineStyle) -> Draw {
  Draw {
    nodes: [
      PathNode.Point { coord: Xy { x: x,     y: y     }, color: stroke, size: stroke_size },
      PathNode.Point { coord: Xy { x: x + w, y: y     }, color: stroke, size: stroke_size },
      PathNode.Point { coord: Xy { x: x + w, y: y + h }, color: stroke, size: stroke_size },
      PathNode.Point { coord: Xy { x: x,     y: y + h }, color: stroke, size: stroke_size },
      PathNode.Point { coord: Xy { x: x,     y: y     }, color: stroke, size: stroke_size }
    ],
    fill: fill,
    line: style
  }
}

// Ellipse approximated with 4 cubic Bezier arcs (standard kappa = 0.5523).
KAPPA = 0.5523f;

pub fn ellipse(cx: single, cy: single, rx: single, ry: single,
               fill: Rgba, stroke: Rgba, stroke_size: single,
               style: LineStyle) -> Draw {
  kx = rx * KAPPA;
  ky = ry * KAPPA;
  Draw {
    nodes: [
      PathNode.Curve { coord: Xy{x:cx,    y:cy-ry}, color:stroke, size:stroke_size,
                       in_handle: Xy{x:kx, y:0.0f}, out_handle: Xy{x:-kx, y:0.0f} },
      PathNode.Curve { coord: Xy{x:cx+rx, y:cy},    color:stroke, size:stroke_size,
                       in_handle: Xy{x:0.0f,y:-ky}, out_handle: Xy{x:0.0f, y:ky} },
      PathNode.Curve { coord: Xy{x:cx,    y:cy+ry}, color:stroke, size:stroke_size,
                       in_handle: Xy{x:kx, y:0.0f}, out_handle: Xy{x:-kx, y:0.0f} },
      PathNode.Curve { coord: Xy{x:cx-rx, y:cy},    color:stroke, size:stroke_size,
                       in_handle: Xy{x:0.0f,y:ky},  out_handle: Xy{x:0.0f, y:-ky} },
      PathNode.Curve { coord: Xy{x:cx,    y:cy-ry}, color:stroke, size:stroke_size,
                       in_handle: Xy{x:kx, y:0.0f}, out_handle: Xy{x:-kx, y:0.0f} }
    ],
    fill: fill,
    line: style
  }
}

// Rounded rect: straight sides + quarter-ellipse corners.
pub fn rounded_rect(x: single, y: single, w: single, h: single, r: single,
                    fill: Rgba, stroke: Rgba, stroke_size: single,
                    style: LineStyle) -> Draw {
  kr = r * KAPPA;
  Draw {
    nodes: [
      PathNode.Point { coord: Xy{x:x+r,   y:y},     color:stroke, size:stroke_size },
      PathNode.Point { coord: Xy{x:x+w-r, y:y},     color:stroke, size:stroke_size },
      PathNode.Curve { coord: Xy{x:x+w,   y:y+r},   color:stroke, size:stroke_size,
                       in_handle:  Xy{x:-kr, y:0.0f}, out_handle: Xy{x:0.0f, y:kr} },
      PathNode.Point { coord: Xy{x:x+w,   y:y+h-r}, color:stroke, size:stroke_size },
      PathNode.Curve { coord: Xy{x:x+w-r, y:y+h},   color:stroke, size:stroke_size,
                       in_handle:  Xy{x:0.0f, y:-kr}, out_handle: Xy{x:-kr, y:0.0f} },
      PathNode.Point { coord: Xy{x:x+r,   y:y+h},   color:stroke, size:stroke_size },
      PathNode.Curve { coord: Xy{x:x,     y:y+h-r}, color:stroke, size:stroke_size,
                       in_handle:  Xy{x:kr, y:0.0f},  out_handle: Xy{x:0.0f, y:-kr} },
      PathNode.Point { coord: Xy{x:x,     y:y+r},   color:stroke, size:stroke_size },
      PathNode.Curve { coord: Xy{x:x+r,   y:y},     color:stroke, size:stroke_size,
                       in_handle:  Xy{x:0.0f, y:kr},  out_handle: Xy{x:kr, y:0.0f} }
    ],
    fill: fill,
    line: style
  }
}

// Arrow shaft + filled triangular head.
pub fn arrow(ax: single, ay: single, bx: single, by: single,
             head_size: single, fill: Rgba, stroke: Rgba,
             stroke_size: single, style: LineStyle) -> Draw {
  ar_dx  = bx - ax;
  ar_dy  = by - ay;
  ar_len = sqrt(ar_dx * ar_dx + ar_dy * ar_dy);
  if ar_len == 0.0f { return Draw { nodes: [], fill: fill, line: style } }
  ar_ux  = ar_dx / ar_len;
  ar_uy  = ar_dy / ar_len;
  // Head base point (set back from tip)
  ar_bx  = bx - ar_ux * head_size;
  ar_by  = by - ar_uy * head_size;
  // Perpendicular for head wings
  ar_wx  = -ar_uy * head_size * 0.4f;
  ar_wy  =  ar_ux * head_size * 0.4f;
  Draw {
    nodes: [
      PathNode.Point { coord: Xy{x:ax, y:ay},            color:stroke, size:stroke_size },
      PathNode.Point { coord: Xy{x:ar_bx, y:ar_by},      color:stroke, size:stroke_size },
      PathNode.Point { coord: Xy{x:ar_bx+ar_wx, y:ar_by+ar_wy}, color:fill, size:1.0f },
      PathNode.Point { coord: Xy{x:bx, y:by},            color:fill,   size:1.0f },
      PathNode.Point { coord: Xy{x:ar_bx-ar_wx, y:ar_by-ar_wy}, color:fill, size:1.0f },
      PathNode.Point { coord: Xy{x:ar_bx+ar_wx, y:ar_by+ar_wy}, color:fill, size:1.0f }
    ],
    fill: fill,
    line: style
  }
}
```

---

## 2D Text rendering

### Library

**`fontdue 0.8`** (pure Rust, compiles to `wasm32`) provides two native ops:
`OpGlyphMetrics` and `OpRasterizeGlyph`.  All layout, alignment, wrapping, and
compositing is implemented in loft.

### `text.loft`

```loft
use lib/graphics/draw;

pub struct Font {
  font_id: integer not null
}

pub enum TextAlign    { Left, Center, Right }
pub enum TextBaseline { Top, Middle, Bottom }

pub struct TextStyle {
  text_font:    Font,
  text_size:    single not null,
  text_color:   Rgba,
  text_align:   TextAlign,
  text_base:    TextBaseline,
  text_spacing: single not null
}

pub struct TextMetrics {
  tm_width:   single not null,
  tm_height:  single not null,
  tm_ascent:  single not null,
  tm_descent: single not null
}

pub fn load_font(path: text) -> Font {
  fid = OpLoadFont(path);
  if fid < 0 { return null }
  Font { font_id: fid }
}

pub fn default_font() -> Font {
  Font { font_id: OpDefaultFont() }
}

// Sum advance widths across all characters.
pub fn measure_text(str: text, style: TextStyle) -> TextMetrics {
  tm_w = 0.0f;
  tm_asc  = 0.0f;
  tm_desc = 0.0f;
  for tm_ch in str {
    tm_gm = OpGlyphMetrics(style.text_font.font_id, tm_ch, style.text_size);
    tm_w += tm_gm.gm_advance + style.text_spacing;
    if tm_gm.gm_ascent  > tm_asc  { tm_asc  = tm_gm.gm_ascent  }
    if tm_gm.gm_descent > tm_desc { tm_desc = tm_gm.gm_descent }
  }
  TextMetrics {
    tm_width:   tm_w,
    tm_height:  tm_asc + tm_desc,
    tm_ascent:  tm_asc,
    tm_descent: tm_desc
  }
}

// Composite a single rasterized glyph onto the canvas at (px, py) = baseline origin.
fn place_glyph(self: &Canvas, bmp: GlyphBitmap, gm: GlyphMetrics,
               px: integer, py: integer, color: Rgba) {
  for pg_row in 0..bmp.gb_height {
    for pg_col in 0..bmp.gb_width {
      pg_cov = bmp.gb_pixels[pg_row * bmp.gb_width + pg_col];
      if pg_cov > 0 {
        pg_a = pg_cov * color.a / 255;
        self.blend_pixel(px + gm.gm_left + pg_col,
                         py - gm.gm_top  + pg_row,
                         Rgba { r: color.r, g: color.g, b: color.b, a: pg_a });
      }
    }
  }
}

// Draw str onto canvas.  Origin (x, y) interpreted by text_align / text_base.
pub fn draw_text(self: &Canvas, str: text, x: single, y: single,
                 style: TextStyle) {
  dt_m = measure_text(str, style);
  // Horizontal alignment
  dt_ox = match style.text_align {
    TextAlign.Left   => x,
    TextAlign.Center => x - dt_m.tm_width / 2.0f,
    TextAlign.Right  => x - dt_m.tm_width,
  };
  // Vertical alignment
  dt_oy = match style.text_base {
    TextBaseline.Top    => y + dt_m.tm_ascent,
    TextBaseline.Middle => y + dt_m.tm_ascent - dt_m.tm_height / 2.0f,
    TextBaseline.Bottom => y,
  };
  dt_cx = dt_ox;
  for dt_ch in str {
    dt_gm  = OpGlyphMetrics(style.text_font.font_id, dt_ch, style.text_size);
    dt_bmp = OpRasterizeGlyph(style.text_font.font_id, dt_ch, style.text_size);
    place_glyph(self, dt_bmp, dt_gm,
                round(dt_cx) as integer, round(dt_oy) as single as integer,
                style.text_color);
    dt_cx += dt_gm.gm_advance + style.text_spacing;
  }
}

// Draw text with automatic line wrapping inside bounding box.
// Returns the y coordinate below the last line rendered.
pub fn draw_text_box(self: &Canvas, str: text,
                     bx: single, by: single, bw: single, bh: single,
                     style: TextStyle) -> single {
  tb_words = str.split(' ');
  tb_cx    = bx;
  tb_cy    = by;
  tb_lh    = measure_text("Ag", style).tm_height * 1.2f;
  for tb_wi in 0..len(tb_words) {
    tb_wm = measure_text(tb_words[tb_wi], style);
    if tb_cx > bx && tb_cx + tb_wm.tm_width > bx + bw {
      tb_cx  = bx;
      tb_cy += tb_lh;
    }
    if tb_cy + tb_lh > by + bh { return tb_cy }
    draw_text(self, tb_words[tb_wi], tb_cx,
              tb_cy + tb_lh * 0.8f, style);
    tb_cx += tb_wm.tm_width + measure_text(" ", style).tm_width;
  }
  tb_cy + tb_lh
}
```

---

## 3D Math — loft implementations

### `math.loft`

A `Mat4` is a `vector<single>` of 16 values in row-major order.
No struct is needed; this avoids field-naming constraints entirely.

```loft
use lib/graphics/mesh;

pub fn mat4_identity() -> vector<single> {
  [1.0f, 0.0f, 0.0f, 0.0f,
   0.0f, 1.0f, 0.0f, 0.0f,
   0.0f, 0.0f, 1.0f, 0.0f,
   0.0f, 0.0f, 0.0f, 1.0f]
}

pub fn mat4_mul(a: vector<single>, b: vector<single>) -> vector<single> {
  result: vector<single> = [];
  for mm_r in 0..4 {
    for mm_c in 0..4 {
      mm_s = 0.0f;
      for mm_k in 0..4 {
        mm_s += a[mm_r * 4 + mm_k] * b[mm_k * 4 + mm_c];
      }
      result += [mm_s];
    }
  }
  result
}

// Multiply mat4 by a homogeneous column vector (x, y, z, w); return Vec4.
pub fn mat4_mul_vec4(m: vector<single>, vx: single, vy: single,
                     vz: single, vw: single) -> Vec4 {
  Vec4 {
    qx: m[0]*vx + m[1]*vy + m[2]*vz  + m[3]*vw,
    qy: m[4]*vx + m[5]*vy + m[6]*vz  + m[7]*vw,
    qz: m[8]*vx + m[9]*vy + m[10]*vz + m[11]*vw,
    qw: m[12]*vx+ m[13]*vy+ m[14]*vz + m[15]*vw
  }
}

// --- Look-at (view matrix) ---
pub fn mat4_look_at(eye: Vec3, target: Vec3, up: Vec3) -> vector<single> {
  // f = normalise(target - eye)
  la_fx = target.vx - eye.vx;
  la_fy = target.vy - eye.vy;
  la_fz = target.vz - eye.vz;
  la_fl = sqrt(la_fx*la_fx + la_fy*la_fy + la_fz*la_fz);
  la_fx /= la_fl; la_fy /= la_fl; la_fz /= la_fl;
  // s = normalise(f × up)
  la_sx = la_fy*up.vz - la_fz*up.vy;
  la_sy = la_fz*up.vx - la_fx*up.vz;
  la_sz = la_fx*up.vy - la_fy*up.vx;
  la_sl = sqrt(la_sx*la_sx + la_sy*la_sy + la_sz*la_sz);
  la_sx /= la_sl; la_sy /= la_sl; la_sz /= la_sl;
  // u = s × f
  la_ux = la_sy*la_fz - la_sz*la_fy;
  la_uy = la_sz*la_fx - la_sx*la_fz;
  la_uz = la_sx*la_fy - la_sy*la_fx;
  [la_sx, la_sy, la_sz, -(la_sx*eye.vx + la_sy*eye.vy + la_sz*eye.vz),
   la_ux, la_uy, la_uz, -(la_ux*eye.vx + la_uy*eye.vy + la_uz*eye.vz),
  -la_fx,-la_fy,-la_fz,   la_fx*eye.vx + la_fy*eye.vy + la_fz*eye.vz,
   0.0f,  0.0f,  0.0f,  1.0f]
}

// --- Perspective projection ---
pub fn mat4_perspective(fov_y: single, aspect: single,
                        near: single, far: single) -> vector<single> {
  pj_f = 1.0f / tan(fov_y / 2.0f);
  pj_d = far - near;
  [pj_f / aspect, 0.0f,  0.0f,                        0.0f,
   0.0f,          pj_f,  0.0f,                        0.0f,
   0.0f,          0.0f, -(far + near) / pj_d,         -2.0f * far * near / pj_d,
   0.0f,          0.0f,  -1.0f,                        0.0f]
}

// --- Euler XYZ → quaternion ---
pub fn euler_to_quat(rx: single, ry: single, rz: single) -> Vec4 {
  eq_cx = cos(rx / 2.0f); eq_sx = sin(rx / 2.0f);
  eq_cy = cos(ry / 2.0f); eq_sy = sin(ry / 2.0f);
  eq_cz = cos(rz / 2.0f); eq_sz = sin(rz / 2.0f);
  Vec4 {
    qx: eq_sx*eq_cy*eq_cz + eq_cx*eq_sy*eq_sz,
    qy: eq_cx*eq_sy*eq_cz - eq_sx*eq_cy*eq_sz,
    qz: eq_cx*eq_cy*eq_sz + eq_sx*eq_sy*eq_cz,
    qw: eq_cx*eq_cy*eq_cz - eq_sx*eq_sy*eq_sz
  }
}

// --- TRS model matrix from Transform ---
// (defined in scene.loft where Transform is declared)
```

The TRS matrix (translation × rotation × scale) is computed in `scene.loft` alongside
`Transform`, where the field names are in scope:

```loft
// Returns the 4×4 model matrix for this transform.
pub fn model_matrix(self: Transform) -> vector<single> {
  mm_q  = euler_to_quat(self.rx, self.ry, self.rz);
  mm_x  = mm_q.qx; mm_y = mm_q.qy; mm_z = mm_q.qz; mm_w = mm_q.qw;
  mm_x2 = mm_x*2.0f; mm_y2 = mm_y*2.0f; mm_z2 = mm_z*2.0f;
  mm_xx = mm_x*mm_x2; mm_yy = mm_y*mm_y2; mm_zz = mm_z*mm_z2;
  mm_xy = mm_x*mm_y2; mm_xz = mm_x*mm_z2; mm_yz = mm_y*mm_z2;
  mm_wx = mm_w*mm_x2; mm_wy = mm_w*mm_y2; mm_wz = mm_w*mm_z2;
  [(1.0f-mm_yy-mm_zz)*self.sx,  (mm_xy-mm_wz)*self.sy,   (mm_xz+mm_wy)*self.sz,  self.tx,
   (mm_xy+mm_wz)*self.sx,       (1.0f-mm_xx-mm_zz)*self.sy, (mm_yz-mm_wx)*self.sz, self.ty,
   (mm_xz-mm_wy)*self.sx,       (mm_yz+mm_wx)*self.sy,   (1.0f-mm_xx-mm_yy)*self.sz, self.tz,
   0.0f, 0.0f, 0.0f, 1.0f]
}
```

---

## 3D Mesh — types

### `mesh.loft`

```loft
pub struct Vec2 { u: single not null, v: single not null }
pub struct Vec3 { vx: single not null, vy: single not null, vz: single not null }
pub struct Vec4 { qx: single not null, qy: single not null,
                  qz: single not null, qw: single not null }

pub struct Vertex {
  pos:    Vec3,
  normal: Vec3,
  uv:     Vec2,
  color:  Rgba
}

pub struct Triangle { ia: integer not null, ib: integer not null, ic: integer not null }

pub struct Mesh {
  mesh_vertices:  vector<Vertex>,
  mesh_triangles: vector<Triangle>
}
```

---

## 3D Scene — types

### `scene.loft`

```loft
use lib/graphics/mesh;
use lib/graphics/math;

pub struct Transform {
  tx: single not null, ty: single not null, tz: single not null,
  rx: single not null, ry: single not null, rz: single not null,
  sx: single not null, sy: single not null, sz: single not null
}

pub fn identity_transform() -> Transform {
  Transform { tx:0.0f, ty:0.0f, tz:0.0f,
              rx:0.0f, ry:0.0f, rz:0.0f,
              sx:1.0f, sy:1.0f, sz:1.0f }
}

pub struct MeshInstance {
  inst_mesh: integer not null,
  inst_mat:  integer not null,
  inst_xf:   Transform
}

pub struct Camera {
  cam_pos:    Vec3,
  cam_target: Vec3,
  cam_up:     Vec3,
  cam_fov:    single,
  cam_near:   single,
  cam_far:    single
}

pub struct Light {
  light_pos:   Vec3,
  light_color: Rgba,
  light_power: single
}

pub struct Scene {
  scene_meshes:    vector<Mesh>,
  scene_materials: vector<Material>,
  scene_objects:   vector<MeshInstance>,
  scene_lights:    vector<Light>,
  scene_camera:    Camera
}
```

---

## Material and texture

### `texture.loft`

```loft
use lib/graphics/draw;

pub enum TextureSource {
  Drawing { drawing_canvas: Canvas },
  Image   { image_file: text }
}

pub struct Material {
  mat_source:    TextureSource,
  mat_tint:      Rgba,
  mat_roughness: single,
  mat_metallic:  single
}
```

Textures are materialized to `Canvas` before upload.  A `Drawing` source is already a
`Canvas`; an `Image` source loads via `load_png`.

---

## GLB export — loft implementation

GLB is produced entirely in loft except for two native ops: `OpSingleBits` (to serialise
floats as bytes) and `OpWriteBytes` (to write the final binary file).

### `glb.loft`

```loft
use lib/graphics/mesh;
use lib/graphics/scene;
use lib/graphics/texture;
use lib/graphics/draw;
use lib/graphics/math;

// --- Binary buffer helpers ---

fn glb_f32(buf: &vector<u8>, v: single) {
  gb_b = OpSingleBits(v);
  buf += [gb_b & 0xFF, (gb_b >> 8) & 0xFF, (gb_b >> 16) & 0xFF, (gb_b >> 24) & 0xFF];
}

fn glb_u32(buf: &vector<u8>, v: integer) {
  buf += [v & 0xFF, (v >> 8) & 0xFF, (v >> 16) & 0xFF, (v >> 24) & 0xFF];
}

fn glb_pad4(buf: &vector<u8>) {
  while len(buf) % 4 != 0 { buf += [0x20]; }   // space padding (JSON chunk)
}

fn glb_pad4_bin(buf: &vector<u8>) {
  while len(buf) % 4 != 0 { buf += [0x00]; }   // zero padding (BIN chunk)
}

// --- Vertex buffer ---

fn glb_vertex_buf(mesh: Mesh) -> vector<u8> {
  gv_buf: vector<u8> = [];
  for gv_vi in 0..len(mesh.mesh_vertices) {
    gv_v = mesh.mesh_vertices[gv_vi];
    glb_f32(gv_buf, gv_v.pos.vx); glb_f32(gv_buf, gv_v.pos.vy); glb_f32(gv_buf, gv_v.pos.vz);
    glb_f32(gv_buf, gv_v.normal.vx); glb_f32(gv_buf, gv_v.normal.vy); glb_f32(gv_buf, gv_v.normal.vz);
    glb_f32(gv_buf, gv_v.uv.u); glb_f32(gv_buf, gv_v.uv.v);
    gv_buf += [gv_v.color.r, gv_v.color.g, gv_v.color.b, gv_v.color.a];
  }
  gv_buf
}

// --- Index buffer ---

fn glb_index_buf(mesh: Mesh) -> vector<u8> {
  gi_buf: vector<u8> = [];
  for gi_ti in 0..len(mesh.mesh_triangles) {
    gi_t = mesh.mesh_triangles[gi_ti];
    glb_u32(gi_buf, gi_t.ia);
    glb_u32(gi_buf, gi_t.ib);
    glb_u32(gi_buf, gi_t.ic);
  }
  gi_buf
}

// --- AABB for an accessor's min/max JSON fields ---

fn glb_aabb(mesh: Mesh) -> text {
  if len(mesh.mesh_vertices) == 0 { return "\"min\":[0,0,0],\"max\":[0,0,0]" }
  ab_v = mesh.mesh_vertices[0];
  ab_mnx = ab_v.pos.vx; ab_mny = ab_v.pos.vy; ab_mnz = ab_v.pos.vz;
  ab_mxx = ab_mnx;      ab_mxy = ab_mny;       ab_mxz = ab_mnz;
  for ab_i in 1..len(mesh.mesh_vertices) {
    ab_p = mesh.mesh_vertices[ab_i].pos;
    if ab_p.vx < ab_mnx { ab_mnx = ab_p.vx } if ab_p.vx > ab_mxx { ab_mxx = ab_p.vx }
    if ab_p.vy < ab_mny { ab_mny = ab_p.vy } if ab_p.vy > ab_mxy { ab_mxy = ab_p.vy }
    if ab_p.vz < ab_mnz { ab_mnz = ab_p.vz } if ab_p.vz > ab_mxz { ab_mxz = ab_p.vz }
  }
  "\"min\":[{ab_mnx},{ab_mny},{ab_mnz}],\"max\":[{ab_mxx},{ab_mxy},{ab_mxz}]"
}

// --- Full save_glb ---

pub fn save_glb(scene: Scene, path: text) -> FileResult {
  // 1. Build BIN chunk: all vertex buffers, index buffers, texture PNGs
  sg_bin: vector<u8> = [];
  sg_vbo_offsets:  vector<integer> = [];
  sg_vbo_lengths:  vector<integer> = [];
  sg_ibo_offsets:  vector<integer> = [];
  sg_ibo_lengths:  vector<integer> = [];
  sg_tex_offsets:  vector<integer> = [];
  sg_tex_lengths:  vector<integer> = [];

  for sg_mi in 0..len(scene.scene_meshes) {
    sg_vbo_offsets += [len(sg_bin)];
    sg_vbo = glb_vertex_buf(scene.scene_meshes[sg_mi]);
    sg_bin += sg_vbo;
    glb_pad4_bin(sg_bin);
    sg_vbo_lengths += [len(sg_vbo)];

    sg_ibo_offsets += [len(sg_bin)];
    sg_ibo = glb_index_buf(scene.scene_meshes[sg_mi]);
    sg_bin += sg_ibo;
    glb_pad4_bin(sg_bin);
    sg_ibo_lengths += [len(sg_ibo)];
  }

  // Embed texture PNGs
  for sg_ti in 0..len(scene.scene_materials) {
    sg_canvas = match scene.scene_materials[sg_ti].mat_source {
      TextureSource.Drawing { drawing_canvas } => drawing_canvas,
      TextureSource.Image   { image_file }     => load_png(image_file),
    };
    sg_tex_offsets += [len(sg_bin)];
    // save canvas to a temp PNG then reload bytes — or use the in-memory PNG encoder
    sg_tmp = "tmp_tex_{sg_ti}.png";
    sg_canvas.save_png(sg_tmp);
    sg_png_canvas = load_png(sg_tmp);   // round-trip to get PNG bytes via file
    // For a cleaner implementation, a native OpCanvasToPngBytes would be added.
    // Here we serialise as RGBA raw bytes as a stand-in.
    for sg_pi in 0..len(sg_png_canvas.canvas_data) {
      sg_px = sg_png_canvas.canvas_data[sg_pi];
      sg_bin += [sg_px.r, sg_px.g, sg_px.b, sg_px.a];
    }
    glb_pad4_bin(sg_bin);
    sg_tex_lengths += [len(sg_bin) - sg_tex_offsets[sg_ti]];
  }

  sg_bin_len = len(sg_bin);

  // 2. Build JSON chunk
  sg_json = "{{\"asset\":{{\"version\":\"2.0\"}},";

  // bufferViews
  sg_json += "\"bufferViews\":[";
  sg_bv_idx = 0;
  sg_bv_sep = "";
  for sg_mj in 0..len(scene.scene_meshes) {
    sg_json += "{sg_bv_sep}{{\"buffer\":0,\"byteOffset\":{sg_vbo_offsets[sg_mj]},\"byteLength\":{sg_vbo_lengths[sg_mj]},\"target\":34962}}";
    sg_bv_sep = ",";
    sg_bv_idx += 1;
    sg_json += ",{{\"buffer\":0,\"byteOffset\":{sg_ibo_offsets[sg_mj]},\"byteLength\":{sg_ibo_lengths[sg_mj]},\"target\":34963}}";
    sg_bv_idx += 1;
  }
  for sg_tj in 0..len(scene.scene_materials) {
    sg_json += ",{{\"buffer\":0,\"byteOffset\":{sg_tex_offsets[sg_tj]},\"byteLength\":{sg_tex_lengths[sg_tj]}}}";
  }
  sg_json += "],";

  // accessors, meshes, materials, images, textures, nodes, scene ...
  // (each section follows the same string-building pattern; abbreviated here)
  sg_json += "\"scene\":0,\"scenes\":[{{\"nodes\":[";
  for sg_oi in 0..len(scene.scene_objects) {
    if sg_oi > 0 { sg_json += "," }
    sg_json += "{sg_oi}";
  }
  sg_json += "]}}],\"nodes\":[";
  for sg_ni in 0..len(scene.scene_objects) {
    sg_obj = scene.scene_objects[sg_ni];
    sg_q   = euler_to_quat(sg_obj.inst_xf.rx, sg_obj.inst_xf.ry, sg_obj.inst_xf.rz);
    if sg_ni > 0 { sg_json += "," }
    sg_json += "{{\"mesh\":{sg_obj.inst_mesh}," +
      "\"translation\":[{sg_obj.inst_xf.tx},{sg_obj.inst_xf.ty},{sg_obj.inst_xf.tz}]," +
      "\"rotation\":[{sg_q.qx},{sg_q.qy},{sg_q.qz},{sg_q.qw}]," +
      "\"scale\":[{sg_obj.inst_xf.sx},{sg_obj.inst_xf.sy},{sg_obj.inst_xf.sz}]}}";
  }
  sg_json += "],\"buffers\":[{{\"byteLength\":{sg_bin_len}}}]}}";

  // 3. Assemble GLB binary
  sg_jbytes: vector<u8> = [];
  for sg_jc in sg_json {
    sg_jbytes += [sg_jc as integer & 0xFF];
  }
  glb_pad4(sg_jbytes);
  sg_json_len = len(sg_jbytes);

  sg_total = 12 + 8 + sg_json_len + 8 + sg_bin_len;
  sg_out: vector<u8> = [];
  // Header: magic "glTF", version 2, total length
  sg_out += [0x67, 0x6C, 0x54, 0x46];   // "glTF"
  glb_u32(sg_out, 2);                    // version
  glb_u32(sg_out, sg_total);
  // JSON chunk
  glb_u32(sg_out, sg_json_len);
  sg_out += [0x4A, 0x53, 0x4F, 0x4E];   // "JSON"
  sg_out += sg_jbytes;
  // BIN chunk
  glb_u32(sg_out, sg_bin_len);
  sg_out += [0x42, 0x49, 0x4E, 0x00];   // "BIN\0"
  sg_out += sg_bin;

  if OpWriteBytes(path, sg_out) { FileResult.Ok } else { FileResult.Other }
}

pub fn save_mesh_glb(mesh: Mesh, mat: Material, xf: Transform, path: text) -> FileResult {
  inst = MeshInstance { inst_mesh: 0, inst_mat: 0, inst_xf: xf };
  scene = Scene {
    scene_meshes:    [mesh],
    scene_materials: [mat],
    scene_objects:   [inst],
    scene_lights:    [],
    scene_camera:    Camera {
      cam_pos:    Vec3{vx:0.0f,vy:0.0f,vz:5.0f},
      cam_target: Vec3{vx:0.0f,vy:0.0f,vz:0.0f},
      cam_up:     Vec3{vx:0.0f,vy:1.0f,vz:0.0f},
      cam_fov: 1.0f, cam_near: 0.1f, cam_far: 100.0f
    }
  };
  save_glb(scene, path)
}
```

---

## OpenGL backend

### Native ops (`opengl.loft`)

One thin native op per GPU call.  No logic in Rust.

```loft
// Window / context
fn OpGlInit(width: integer, height: integer) -> boolean;
#rust"renderer::gl_init(@width, @height)"
fn OpGlShutdown();
#rust"renderer::gl_shutdown();"
fn OpGlPollEvents() -> boolean;
#rust"renderer::gl_poll_events()"
fn OpGlSwapBuffers();
#rust"renderer::gl_swap_buffers();"
fn OpGlClear(r: single, g: single, b: single);
#rust"renderer::gl_clear(@r, @g, @b);"

// Mesh upload — returns VAO handle
fn OpGlUploadMesh(verts: vector<u8>, indices: vector<u8>) -> integer;
#rust"renderer::gl_upload_mesh(&@verts, &@indices)"

// Texture upload — returns texture handle
fn OpGlUploadTexture(canvas: Canvas) -> integer;
#rust"renderer::gl_upload_texture(&@canvas)"
fn OpGlReleaseTexture(id: integer);
#rust"renderer::gl_release_texture(@id);"

// Draw one mesh
fn OpGlBindTexture(id: integer);
#rust"renderer::gl_bind_texture(@id);"
fn OpGlSetMat4(location: integer, mat: vector<single>);
#rust"renderer::gl_set_mat4(@location, &@mat);"
fn OpGlSetVec3(location: integer, x: single, y: single, z: single);
#rust"renderer::gl_set_vec3(@location, @x, @y, @z);"
fn OpGlSetFloat(location: integer, v: single);
#rust"renderer::gl_set_float(@location, @v);"
fn OpGlDrawMesh(vao: integer, index_count: integer);
#rust"renderer::gl_draw_mesh(@vao, @index_count);"

// Shader uniform locations (looked up once after init)
fn OpGlUniformLoc(program: integer, name: text) -> integer;
#rust"renderer::gl_uniform_loc(@program, @name)"
fn OpGlGetProgram() -> integer;
#rust"renderer::gl_get_program()"
```

### Loft render loop (`opengl.loft`, continued)

```loft
pub enum Backend { OpenGl, WebGl, Glb }

pub fn init_renderer(backend: Backend, width: integer, height: integer) -> boolean {
  match backend {
    Backend.OpenGl => OpGlInit(width, height),
    _              => false,
  }
}

pub fn shutdown() { OpGlShutdown(); }
pub fn poll_events() -> boolean { OpGlPollEvents() }

// Upload all meshes and materials in the scene; store handles back in the scene.
// Returns a vector of VAO handles (one per mesh) and texture handles (one per material).
pub fn upload_scene(scene: Scene) -> vector<integer> {
  us_vao_handles: vector<integer> = [];
  for us_mi in 0..len(scene.scene_meshes) {
    us_vb = glb_vertex_buf(scene.scene_meshes[us_mi]);
    us_ib = glb_index_buf(scene.scene_meshes[us_mi]);
    us_vao_handles += [OpGlUploadMesh(us_vb, us_ib)];
  }
  us_tex_handles: vector<integer> = [];
  for us_ti in 0..len(scene.scene_materials) {
    us_canvas = match scene.scene_materials[us_ti].mat_source {
      TextureSource.Drawing { drawing_canvas } => drawing_canvas,
      TextureSource.Image   { image_file }     => load_png(image_file),
    };
    us_tex_handles += [OpGlUploadTexture(us_canvas)];
  }
  us_vao_handles += us_tex_handles;
  us_vao_handles
}

// Render one frame.  handles = result of upload_scene.
pub fn render(scene: Scene, handles: vector<integer>, width: integer, height: integer) {
  OpGlClear(0.1f, 0.1f, 0.12f);
  rl_prog   = OpGlGetProgram();
  rl_n_mesh = len(scene.scene_meshes);
  rl_aspect = width as single / height as single;
  rl_view   = mat4_look_at(scene.scene_camera.cam_pos,
                            scene.scene_camera.cam_target,
                            scene.scene_camera.cam_up);
  rl_proj   = mat4_perspective(scene.scene_camera.cam_fov, rl_aspect,
                                scene.scene_camera.cam_near,
                                scene.scene_camera.cam_far);
  rl_vp = mat4_mul(rl_proj, rl_view);

  for rl_oi in 0..len(scene.scene_objects) {
    rl_obj  = scene.scene_objects[rl_oi];
    rl_model = model_matrix(rl_obj.inst_xf);
    rl_mvp   = mat4_mul(rl_vp, rl_model);
    rl_vao   = handles[rl_obj.inst_mesh];
    rl_tex   = handles[rl_n_mesh + rl_obj.inst_mat];
    rl_n_idx = len(scene.scene_meshes[rl_obj.inst_mesh].mesh_triangles) * 3;
    rl_mat   = scene.scene_materials[rl_obj.inst_mat];

    OpGlBindTexture(rl_tex);
    OpGlSetMat4(OpGlUniformLoc(rl_prog, "u_mvp"), rl_mvp);
    if len(scene.scene_lights) > 0 {
      rl_l = scene.scene_lights[0];
      OpGlSetVec3(OpGlUniformLoc(rl_prog, "u_light_pos"),
                  rl_l.light_pos.vx, rl_l.light_pos.vy, rl_l.light_pos.vz);
      OpGlSetFloat(OpGlUniformLoc(rl_prog, "u_light_power"), rl_l.light_power);
    }
    OpGlSetFloat(OpGlUniformLoc(rl_prog, "u_roughness"), rl_mat.mat_roughness);
    OpGlDrawMesh(rl_vao, rl_n_idx);
  }
  OpGlSwapBuffers();
}
```

---

## WebGL backend

The WebGL backend uses identical loft render logic (`render`, `upload_scene`, all math).
Only the native op names differ (`OpWgl*` instead of `OpGl*`).

```loft
// webgl.loft — same structure as opengl.loft, different op bodies
fn OpWglInit(width: integer, height: integer) -> boolean;
#rust"renderer::wgl_init(@width, @height)"   // acquires WebGL2 context from DOM
fn OpWglShutdown();
#rust"renderer::wgl_shutdown();"
fn OpWglPollEvents() -> boolean;
#rust"renderer::wgl_poll_events()"
fn OpWglSwapBuffers();
#rust"renderer::wgl_swap_buffers();"   // no-op — browser auto-presents
fn OpWglClear(r: single, g: single, b: single);
#rust"renderer::wgl_clear(@r, @g, @b);"
fn OpWglUploadMesh(verts: vector<u8>, indices: vector<u8>) -> integer;
#rust"renderer::wgl_upload_mesh(&@verts, &@indices)"
fn OpWglUploadTexture(canvas: Canvas) -> integer;
#rust"renderer::wgl_upload_texture(&@canvas)"
fn OpWglBindTexture(id: integer);
#rust"renderer::wgl_bind_texture(@id);"
fn OpWglSetMat4(location: integer, mat: vector<single>);
#rust"renderer::wgl_set_mat4(@location, &@mat);"
fn OpWglSetVec3(location: integer, x: single, y: single, z: single);
#rust"renderer::wgl_set_vec3(@location, @x, @y, @z);"
fn OpWglSetFloat(location: integer, v: single);
#rust"renderer::wgl_set_float(@location, @v);"
fn OpWglDrawMesh(vao: integer, index_count: integer);
#rust"renderer::wgl_draw_mesh(@vao, @index_count);"
fn OpWglUniformLoc(program: integer, name: text) -> integer;
#rust"renderer::wgl_uniform_loc(@program, @name)"
fn OpWglGetProgram() -> integer;
#rust"renderer::wgl_get_program()"
```

The loft `render` function is reused as-is with the `Backend.WebGl` branch substituting
`OpWgl*` calls for `OpGl*`.  In practice a `use` alias or a
`fn gl_draw_mesh(vao, count)` wrapper that dispatches on the active backend makes both
backends share a single loft render function body.

---

## Rust integration notes

Rust provides only true FFI/codec operations.  There is no computational logic in Rust.

### Modules

| Module | Content |
|---|---|
| `src/drawing.rs` | Font registry, `glyph_metrics`, `rasterize_glyph`, PNG codec wrappers |
| `src/renderer/mod.rs` | Backend-agnostic init/shutdown; `single_bits`; `write_bytes` |
| `src/renderer/opengl.rs` | OpenGL 3.3 core: window, VBO/VAO, texture, shader, draw, uniforms |
| `src/renderer/webgl.rs` | WebGL 2 via `web-sys`: same surface as OpenGL module; WASM only |

### Crate dependencies

| Crate | Purpose | Feature gate |
|---|---|---|
| `fontdue 0.8` | TTF/OTF rasterization (pure Rust, works on wasm32) | — (always) |
| `glfw` | GLFW window + OpenGL context | `opengl` |
| `gl` | OpenGL function loader | `opengl` |
| `web-sys` | WebGL 2 + DOM bindings | `webgl` |
| `wasm-bindgen` | WASM/JS bridge | `webgl` |

```toml
[features]
opengl  = ["dep:glfw", "dep:gl"]
webgl   = ["dep:web-sys", "dep:wasm-bindgen"]
default = ["opengl"]

[dependencies]
fontdue = "0.8"
```

### What Rust does NOT contain

- Matrix math (look-at, perspective, TRS, quaternion) — all loft
- Scene traversal and draw call setup — all loft
- GLB JSON building — all loft
- GLB BIN buffer layout — all loft
- Anti-aliased line rasterizer — all loft
- Bezier subdivision — all loft
- Scanline fill — all loft
- All 2D primitives — all loft
- Text layout, alignment, wrapping, compositing — all loft

---

## Usage examples

### Draw a rounded button and save as PNG

```loft
use lib/graphics/draw;
use lib/graphics/primitives;

fn main() {
  c = canvas(256);
  bg     = Rgba { r: 60,  g: 120, b: 200, a: 255 };
  border = Rgba { r: 255, g: 255, b: 255, a: 200 };
  none   = Rgba { r: 0,   g: 0,   b: 0,   a: 0 };
  c.draw(rounded_rect(8.0f, 8.0f, 240.0f, 80.0f, 12.0f,
                      bg, border, 2.0f, LineStyle.Straight));
  c.save_png("button.png");
}
```

### Export to GLB

```loft
use lib/graphics/mesh;
use lib/graphics/scene;
use lib/graphics/glb;

fn main() {
  scene = /* ... build scene ... */;
  result = save_glb(scene, "output/scene.glb");
  if !result.ok() { println("GLB export failed"); }
}
```

### Render a 3D scene (OpenGL)

```loft
use lib/graphics/draw;
use lib/graphics/primitives;
use lib/graphics/mesh;
use lib/graphics/scene;
use lib/graphics/texture;
use lib/graphics/opengl;
use lib/graphics/math;

fn main() {
  init_renderer(Backend.OpenGl, 1280, 720);

  tex = canvas(256);
  tex.draw(rect(0.0f, 0.0f, 256.0f, 256.0f,
    Rgba{r:80,g:160,b:80,a:255}, Rgba{r:0,g:0,b:0,a:0}, 0.0f, LineStyle.Straight));

  scene = Scene {
    scene_meshes:    [make_box()],
    scene_materials: [Material {
      mat_source: TextureSource.Drawing { drawing_canvas: tex },
      mat_tint: Rgba{r:255,g:255,b:255,a:255}, mat_roughness: 0.6f, mat_metallic: 0.0f
    }],
    scene_objects: [MeshInstance {
      inst_mesh: 0, inst_mat: 0, inst_xf: identity_transform()
    }],
    scene_lights: [Light {
      light_pos: Vec3{vx:5.0f,vy:8.0f,vz:4.0f},
      light_color: Rgba{r:255,g:255,b:255,a:255}, light_power: 1.0f
    }],
    scene_camera: Camera {
      cam_pos: Vec3{vx:3.0f,vy:2.0f,vz:5.0f},
      cam_target: Vec3{vx:0.0f,vy:0.0f,vz:0.0f},
      cam_up: Vec3{vx:0.0f,vy:1.0f,vz:0.0f},
      cam_fov: 1.0f, cam_near: 0.1f, cam_far: 100.0f
    }
  };

  handles = upload_scene(scene);
  while poll_events() {
    render(scene, handles, 1280, 720);
  }
  shutdown();
}
```

---

## Implementation constraints

### ~~Field uniqueness per file~~ (resolved)

Field lookups are type-scoped — struct field names may overlap safely.
The prefixed naming in the table below is retained for readability, not necessity.

| Struct | Prefix | Example fields |
|---|---|---|
| `Vec2` | `u`, `v` | `u`, `v` |
| `Vec3` | `v` + axis | `vx`, `vy`, `vz` |
| `Vec4` | `q` + axis | `qx`, `qy`, `qz`, `qw` |
| `Transform` | axis letter | `tx`, `ty`, `rx`, `ry`, `sx`, `sy` |
| `Camera` | `cam_` | `cam_pos`, `cam_fov` |
| `Light` | `light_` | `light_pos`, `light_color` |
| `MeshInstance` | `inst_` | `inst_mesh`, `inst_xf` |
| `Scene` | `scene_` | `scene_meshes`, `scene_camera` |
| `Canvas` | `canvas_` | `canvas_size`, `canvas_data` |
| `Mesh` | `mesh_` | `mesh_vertices`, `mesh_triangles` |
| `Material` | `mat_` | `mat_source`, `mat_tint` |
| `GlyphMetrics` | `gm_` | `gm_advance`, `gm_ascent` |
| `GlyphBitmap` | `gb_` | `gb_width`, `gb_pixels` |
| `TextStyle` | `text_` | `text_font`, `text_size` |
| `TextMetrics` | `tm_` | `tm_width`, `tm_ascent` |

### Flat namespace — loop variable uniqueness

All functions in a `.loft` file share one global variable namespace (see
[LOFT.md](LOFT.md)).  Every loop variable in the library
uses a function-specific prefix (e.g. `wu_`, `bz_`, `sf_`, `sg_`, `rl_`, `us_`) to
prevent collisions across functions.

### `single` vs `float`

All 3D and canvas coordinates use `single` (32-bit) for GPU compatibility and performance.
`float` (64-bit) is only used when intermediate precision demands it (none in this library).

### Backend feature gates

```toml
[features]
opengl  = ["dep:glfw", "dep:gl"]
webgl   = ["dep:web-sys", "dep:wasm-bindgen"]
default = ["opengl"]
```

A headless (GLB-only) build needs neither feature.

---

## Known issues

### Store leaks in interpreter mode

Running OpenGL examples with `--interpret` produces store-not-freed warnings on exit:
```
Warning: store 1 not freed at program exit (possible resource leak)
Warning: store 3 not freed at program exit (possible resource leak)
...
```

Each render frame allocates temporary stores for intermediate values (vertices,
matrices, shader uniforms) that are not freed before program exit. This is related
to issue #117 (struct-returning functions leak callee stores after deep copy) but
is amplified by the render loop — one or more stores leak per frame.

**Impact:** Informational only. Memory grows over time but is reclaimed on exit.
Not visible in compiled mode (native codegen uses stack allocation).

**Fix direction:** Track store ownership through the render loop and ensure
per-frame temporaries are freed. May require the interpreter to recognise
single-use return stores and free them after consumption.
