
# Game Asset Pipeline

How to build a loft game with AI-generated placeholder art and sound, then
replace them with hand-crafted assets using external tools.

---

## Overview

```
Phase 1: Claude writes game      Phase 2: Artist creates       Phase 3: Integrate
─────────────────────────────     ──────────────────────────     ──────────────────
Procedural sprites (fill_rect)    Pixel art in Aseprite/etc.    load_sprite_sheet()
  → build_atlas() fallback          → atlas.png                   tries PNG first
Procedural SFX (sfx_beep)        SFX in jsfxr + Audacity       audio_load() with
  → synth fallback                  → sfx/*.wav                   synth fallback
Hardcoded note arrays             Music in LMMS/BeepBox         audio_load() with
  → sequencer fallback              → music/*.ogg                 sequencer fallback
```

The procedural assets are never deleted.  They serve as the **fallback** for
headless CI, tests, and first-run without asset files.

---

## Phase 1 — Claude builds the prototype

### Sprites

Build the atlas in code using `canvas()`, `fill_rect`, `fill_circle`, etc.
Use a consistent grid (e.g. 4 columns × 5 rows of 32×32 cells) and document
the sprite index map at the top of the file:

```loft
// SPRITE MAP (4 cols × 5 rows, 32×32 cells):
//   0: ic_extend    1: ic_explode   2: ic_balloon   3: ic_newrow
//   4: ic_multi     5: ic_fire      6: ic_shield    7: pk_extend
//  ...
//  16: ball_normal 17: ball_sq1    18: ball_sq2    19: ball_release
```

Export the procedural atlas as a PNG reference for the artist:

```loft
build_atlas().save_png("reference_atlas.png");
```

### Sound effects

Use the procedural synthesis functions (`sfx_beep`, `sfx_chirp`, `sfx_descend`,
`sfx_bounce`, `sfx_noise`) for every game event.  Document each call's purpose:

```loft
// Brick break — pitch rises with combo for "streak" feel
graphics::sfx_beep(400.0f + combo as single * 40.0f, 0.06f, 0.25f);
```

### Music

Hardcode note-frequency arrays and step through them with `sfx_beep`:

```loft
TRACK_A = [523, 659, 784, 1047, ...];  // C5 E5 G5 C6
```

This is ugly but it produces playable audio with zero external files.

### Deliverable

A fully playable game where every asset is generated in code.  No external
files required.  This is the version Claude hands off.

---

## Phase 2 — Create real assets with external tools

### 2a. Sprite art

**Recommended tools (pick one):**

| Tool | License | Cost | Strengths |
|---|---|---|---|
| [Aseprite](https://www.aseprite.org/) | Source-available | $20 (or build from source) | Industry standard pixel art; animation timeline, onion skin, sheet export with JSON metadata |
| [Pixelorama](https://orama-interactive.itch.io/pixelorama) | MIT | Free | Good pixel art editor with animation; Godot-based, cross-platform |
| [LibreSprite](https://libresprite.github.io/) | GPL | Free | Aseprite fork (older); functional but less maintained |
| [GIMP](https://www.gimp.org/) | GPL | Free | Capable but no animation timeline; use for touch-ups, not primary pixel art |

**Workflow:**

1. Run the game once to export `reference_atlas.png` (or screenshot the `I`
   cheat-key diagnostic overlay which shows all sprites labelled by index).
2. Open in Aseprite/Pixelorama.  Create a new file at the atlas dimensions
   (e.g. 128×160).  Enable the grid overlay at the cell size (32×32).
3. Paint over each cell, using the reference as a placement guide.
4. Keep transparent background (alpha=0) for empty space.
5. **Export as a single PNG:** `atlas.png` (no JSON needed — the grid layout
   is the contract).

**Constraints to communicate to the artist:**

- Grid is `cols × rows` cells of `W × H` pixels (e.g. 4×5 of 32×32).
- Sprite indices are row-major: index 0 = top-left, then left-to-right,
  then next row.
- Every sprite must stay within its cell boundaries.
- The game code scales sprites to arbitrary screen sizes — draw at native
  resolution and let the GPU handle scaling.

### 2b. Sound effects

**Recommended tools:**

| Tool | License | Cost | Strengths |
|---|---|---|---|
| [jsfxr](https://sfxr.me/) | MIT | Free, browser | One-click retro sounds; categories match game needs (pickup, hit, explosion) |
| [Audacity](https://www.audacityteam.org/) | GPL | Free | Post-process: normalize, trim, layer, convert formats |
| [Bfxr](https://www.bfxr.net/) | Free | Free, browser | jsfxr variant with more waveforms and mutation |

**Workflow:**

1. For each game event, generate a candidate in jsfxr.
2. Tweak parameters; re-export until it feels right.
3. Export as **WAV, mono, 22050 Hz** (matches `SFX_RATE` in graphics.loft).
4. Optionally post-process in Audacity: trim silence, normalize to −3 dB.
5. Save as `sfx/<event_name>.wav`.

**Sound map** (events to replace):

| Event | Current synth | File |
|---|---|---|
| Wall bounce | `sfx_beep(600, 0.03, 0.1)` | `sfx/wall_bounce.wav` |
| Brick break | `sfx_beep(400+combo*40, 0.06, 0.25)` | `sfx/brick_hit.wav` (or multiple pitch variants) |
| Paddle hit | `sfx_bounce(180, 0.3)` | `sfx/paddle_hit.wav` |
| Pickup collected | `sfx_chirp(400→900, 0.12, 0.3)` | `sfx/pickup.wav` |
| Life lost | `sfx_descend(500→150, 0.4, 0.4)` | `sfx/life_lost.wav` |
| Level clear | (flash only) | `sfx/level_clear.wav` |
| Balloon pop | (flash + shake only) | `sfx/balloon_pop.wav` |

### 2c. Music

**Recommended tools:**

| Tool | License | Cost | Strengths |
|---|---|---|---|
| [BeepBox](https://www.beepbox.co/) | MIT | Free, browser | Zero install; pattern-based chiptune; shareable URLs; export WAV |
| [Bosca Ceoil](https://boscaceoil.net/) | Free | Free | Purpose-built for game chiptune; pattern-based, 8-bit focused |
| [LMMS](https://lmms.io/) | GPL | Free | Full DAW with chiptune synths (TripleOscillator, BitInvader, FreeBoy) |
| [FamiStudio](https://famistudio.org/) | MIT | Free | NES-authentic chiptune; NSF export + WAV |

**Workflow:**

1. Use the prototype note arrays as a melodic starting point — the
   frequencies map to standard notes:

   ```
   523=C5  587=D5  659=E5  698=F5  784=G5  880=A5  988=B5  1047=C6
   440=A4  494=B4  349=F4  392=G4
   ```

2. Compose in BeepBox/LMMS with proper instruments, harmony, and rhythm.
3. Export loops as **OGG** (smaller than WAV; `audio_load` supports both).
4. Keep loops short (4–8 seconds) to minimize file size.
5. Save as `music/track_a.ogg`, `music/track_b.ogg`, etc.

---

## Phase 3 — Integrate assets into the game

### File layout

```
lib/graphics/examples/
  25-brick-buster.loft
  assets/
    atlas.png                   # sprite sheet (same grid as build_atlas)
    atlas.aseprite              # source file (not shipped, keep for edits)
  sfx/
    brick_hit.wav
    wall_bounce.wav
    paddle_hit.wav
    pickup.wav
    life_lost.wav
    level_clear.wav
    balloon_pop.wav
  music/
    track_a.ogg
    track_b.ogg
    track_c.ogg
```

### Code changes

See [Code changes required](#code-changes-required) below for the exact steps.
The core idea: every asset load has a fallback path so the game runs with or
without external files.

---

## Code changes required

### Step 1 — Add `load_sprite_sheet` to the graphics library

`create_sprite_sheet` only accepts a `Canvas`.  To load a PNG atlas we need a
variant that takes a file path and falls back to a Canvas.

Add to `lib/graphics/src/graphics.loft`:

```loft
/// Load a sprite sheet from a PNG file.  Falls back to the given Canvas
/// if the file does not exist or cannot be loaded.
/// `cols` and `rows` define the grid subdivision.
pub fn load_sprite_sheet(path: text, fallback: const Canvas,
                         cols: integer, rows: integer, vao: integer) -> SpriteSheet {
  ss_tex = gl_load_texture(path);
  if ss_tex > 0 {
    ss_shader = gl_create_shader(SPRITE_VERT, SPRITE_FRAG);
    return SpriteSheet {
      ss_tex: ss_tex,
      ss_shader: ss_shader,
      ss_vao: vao,
      ss_cols: cols,
      ss_rows: rows,
      ss_cell_w: fallback.width / cols,
      ss_cell_h: fallback.height / rows,
    };
  }
  create_sprite_sheet(fallback, cols, rows, vao)
}
```

### Step 2 — Use `load_sprite_sheet` in Brick Buster

In `25-brick-buster.loft`, change the atlas init from:

```loft
br_atlas = build_atlas();
br_sheet = graphics::create_sprite_sheet(br_atlas, 4, 5, graphics::painter_vao(br_painter));
```

to:

```loft
br_atlas = build_atlas();
br_sheet = graphics::load_sprite_sheet("assets/atlas.png", br_atlas, 4, 5,
                                       graphics::painter_vao(br_painter));
```

No other rendering code changes — same sprite indices, same draw calls.

### Step 3 — Add sound-loading helper

Add a helper that loads a WAV/OGG and falls back to a synth function:

```loft
// Load a sound file; returns 0 if the file is missing.
fn load_sfx(path: text) -> integer {
  snd = graphics::audio_load(path);
  if snd { return snd; }
  0
}
```

### Step 4 — Load sound effects at init

After the font loading block, add:

```loft
br_snd_wall    = load_sfx("sfx/wall_bounce.wav");
br_snd_brick   = load_sfx("sfx/brick_hit.wav");
br_snd_paddle  = load_sfx("sfx/paddle_hit.wav");
br_snd_pickup  = load_sfx("sfx/pickup.wav");
br_snd_lose    = load_sfx("sfx/life_lost.wav");
br_snd_level   = load_sfx("sfx/level_clear.wav");
br_snd_balloon = load_sfx("sfx/balloon_pop.wav");
```

### Step 5 — Replace inline synth calls with play-or-fallback

Create a helper:

```loft
fn play_or(snd: integer, vol: float, fallback_fn: fn()) {
  if snd { graphics::audio_play(snd, vol); }
  else   { fallback_fn(); }
}
```

Then replace each synth call.  For example, wall bounce:

**Before:**
```loft
graphics::sfx_beep(600.0f, 0.03f, 0.1f);
```

**After:**
```loft
if br_snd_wall { graphics::audio_play(br_snd_wall, 0.1); }
else { graphics::sfx_beep(600.0f, 0.03f, 0.1f); }
```

Apply the same pattern to all six sound events.  The brick-hit sound with
rising pitch can use a single WAV at fixed pitch (the pitch ramp was a
placeholder effect) or load multiple variants (`brick_hit_1.wav` through
`brick_hit_5.wav`) indexed by combo count.

### Step 6 — Load music tracks at init

```loft
br_mus_files = [
  graphics::audio_load("music/track_a.ogg"),
  graphics::audio_load("music/track_b.ogg"),
  graphics::audio_load("music/track_c.ogg"),
];
br_mus_use_files = br_mus_files[0] > 0 || br_mus_files[1] > 0 || br_mus_files[2] > 0;
br_mus_sink = 0;
```

### Step 7 — Replace the note-by-note sequencer with track playback

In the music sequencer block (lines 888–939), wrap the existing note-by-note
logic in an `if !br_mus_use_files` guard and add the file-based path:

```loft
if br_mus_use_files {
  // File-based music: play whole tracks
  if !br_mus_resting && br_mus_played < MUSIC_TRACKS {
    if br_mus_files[br_mus_song] > 0 {
      br_mus_sink = graphics::audio_play(br_mus_files[br_mus_song], MUSIC_VOL as float);
    }
    // Estimate track duration from the original note count × note duration
    br_mus_t = MUSIC_TRACK_LEN as single * (if br_mus_song == 0 { TRACK_A_DUR }
               else if br_mus_song == 1 { TRACK_B_DUR } else { TRACK_C_DUR });
    br_mus_played += 1;
    br_mus_song = (br_mus_song + 1) % MUSIC_TRACKS;
    br_mus_resting = true;
    br_mus_t = br_mus_t + 3.0f + (((br_now / 23l) as integer) % 5) as single;
  }
} else {
  // ... existing note-by-note sequencer (unchanged) ...
}
```

### Step 8 — Export reference atlas for artists

Add a one-time export behind the `I` cheat key or a separate flag:

```loft
// In the cheat-key block, add:
// E (101) — export procedural atlas as PNG for artist reference
br_cheat_e_down = graphics::gl_key_pressed(101);
br_cheat_e_pressed = br_cheat_e_down && !br_cheat_e_was_down;
br_cheat_e_was_down = br_cheat_e_down;
if br_cheat_e_pressed {
  br_atlas.save_png("reference_atlas.png");
  println("Exported reference_atlas.png");
}
```

---

## Summary of changes

| # | What | Where | Size |
|---|---|---|---|
| 1 | Add `load_sprite_sheet` function | `graphics.loft` | ~15 lines |
| 2 | Use `load_sprite_sheet` in game init | `25-brick-buster.loft` | 2 lines changed |
| 3 | Add `load_sfx` helper | `25-brick-buster.loft` | 4 lines |
| 4 | Load sound files at init | `25-brick-buster.loft` | 7 lines added |
| 5 | Wrap each synth call in play-or-fallback | `25-brick-buster.loft` | ~12 lines changed |
| 6 | Load music files at init | `25-brick-buster.loft` | 5 lines added |
| 7 | Add file-based music path alongside sequencer | `25-brick-buster.loft` | ~15 lines added |
| 8 | Atlas export cheat key | `25-brick-buster.loft` | 6 lines added |

Total: ~65 lines of new/changed code.  Zero lines deleted — every procedural
asset becomes the fallback path.
