// loft-gl.js — WebGL2 bridge for loft WASM games
// Provides the loftHost.gl_* methods that the loft interpreter calls via host_call.
// Usage:
//   import { initLoftGL } from './loft-gl.js';
//   const loftGL = initLoftGL(document.getElementById('my-canvas'));
//   // loftGL.shouldClose, loftGL.assets, loftGL.gl

/**
 * Initialize the WebGL2 bridge on a canvas element.
 * Sets up window.loftHost with all gl_* methods.
 * Returns a control object for the game loop.
 *
 * @param {HTMLCanvasElement} canvas
 * @returns {{ gl: WebGL2RenderingContext, shouldClose: boolean, assets: Object }}
 */
export function initLoftGL(canvas) {
  let gl = canvas.getContext('webgl2', { antialias: true, alpha: false });
  if (!gl) throw new Error('WebGL2 not available');

  let programs = [];
  let vaos = [];
  let textures = [];
  let fbos = [];

  // ── GL capability / blend / draw mode maps ──────────────────────────────

  function glCap(cap) {
    switch (cap) {
      case 1: return gl.DEPTH_TEST;
      case 2: return gl.BLEND;
      case 3: return gl.CULL_FACE;
      default: return cap;
    }
  }

  function glBlendFactor(f) {
    switch (f) {
      case 0: return gl.ZERO;
      case 1: return gl.ONE;
      case 2: return gl.SRC_ALPHA;
      case 3: return gl.ONE_MINUS_SRC_ALPHA;
      case 4: return gl.DST_ALPHA;
      case 5: return gl.ONE_MINUS_DST_ALPHA;
      default: return f;
    }
  }

  function glDrawMode(mode) {
    switch (mode) {
      case 0: return gl.TRIANGLES;
      case 1: return gl.LINES;
      case 2: return gl.POINTS;
      default: return gl.TRIANGLES;
    }
  }

  // ── Input state ─────────────────────────────────────────────────────────

  const keys = new Set();
  let mouseX = 0, mouseY = 0, mouseBtn = 0;
  // Initiative 03 Phase 1: scroll-wheel accumulator — matches the
  // native WHEEL_ACCUM thread-local.  Positive = scroll up.
  let wheelAccum = 0;

  function mapKeyCode(code) {
    // Loft key constants use lowercase ASCII: KEY_W=119 ('w'), KEY_A=97 ('a')
    if (code.startsWith('Key')) return code.charCodeAt(3) + 32;
    if (code.startsWith('Digit')) return code.charCodeAt(5);
    const special = {
      ArrowUp: 128, ArrowDown: 129, ArrowLeft: 130, ArrowRight: 131,
      ShiftLeft: 132, ShiftRight: 132, ControlLeft: 133, ControlRight: 133,
      Space: 32, Enter: 13, Escape: 27, Tab: 9,
      BracketLeft: 91, BracketRight: 93,
      // F-key range 135..146 — mirrors the native NamedKey::F1..F12 mapping.
      F1: 135, F2: 136, F3: 137, F4: 138, F5: 139, F6: 140,
      F7: 141, F8: 142, F9: 143, F10: 144, F11: 145, F12: 146,
    };
    return special[code] || 0;
  }

  canvas.tabIndex = 0;
  canvas.addEventListener('keydown', e => { keys.add(mapKeyCode(e.code)); e.preventDefault(); });
  canvas.addEventListener('keyup', e => keys.delete(mapKeyCode(e.code)));
  canvas.addEventListener('mousemove', e => {
    const r = canvas.getBoundingClientRect();
    mouseX = e.clientX - r.left;
    mouseY = e.clientY - r.top;
  });
  canvas.addEventListener('mousedown', e => { mouseBtn |= (1 << e.button); });
  canvas.addEventListener('mouseup', e => { mouseBtn &= ~(1 << e.button); });
  // Wheel: browsers report deltaY inverted (down = positive).  Flip to
  // match native (up = positive) and quantise at 100 px/tick (the
  // default browser wheel line height).
  canvas.addEventListener('wheel', e => {
    const ticks = -Math.sign(e.deltaY) * Math.max(1, Math.round(Math.abs(e.deltaY) / 100));
    wheelAccum += ticks;
    e.preventDefault();
  }, { passive: false });

  // ── Control object ──────────────────────────────────────────────────────

  const ctrl = {
    gl,
    shouldClose: false,
    /** Pre-loaded binary assets keyed by filename. Set .image to an HTMLImageElement. */
    assets: {},
  };

  // ── loftHost interface ──────────────────────────────────────────────────

  window.loftHost = window.loftHost || {};
  Object.assign(window.loftHost, {

    // Time — milliseconds since epoch (used by loft ticks())
    time_now() { return Date.now(); },
    time_ticks() { return performance.now(); },

    // Window lifecycle
    gl_create_window(w, h, _title) {
      canvas.width = w;
      canvas.height = h;
      programs = [];
      vaos = [];
      textures = [];
      fbos = [];
      gl.viewport(0, 0, w, h);
      // Match native: depth test enabled by default
      gl.enable(gl.DEPTH_TEST);
      return true;
    },

    // Initiative 03 Phase 0: browser-side fullscreen.
    //
    // Browsers gate `requestFullscreen()` behind a user-gesture event;
    // calling it from module init (which is typically triggered by the
    // HTML loader, not a click) fails silently.  The stub therefore:
    //   1. Sets the canvas to the current viewport dimensions so the
    //      3D scene renders at the right aspect ratio.
    //   2. Attempts `requestFullscreen()`.  A SecurityError from
    //      outside a user gesture is non-fatal — the user can bind a
    //      button that calls `gl_create_fullscreen_window` again to
    //      enter real fullscreen on demand.
    gl_create_fullscreen_window(_title) {
      const w = window.innerWidth || 1024;
      const h = window.innerHeight || 768;
      canvas.width = w;
      canvas.height = h;
      programs = [];
      vaos = [];
      textures = [];
      fbos = [];
      gl.viewport(0, 0, w, h);
      gl.enable(gl.DEPTH_TEST);
      if (document.documentElement.requestFullscreen) {
        document.documentElement.requestFullscreen().catch(() => {
          // Non-fatal: caller likely invoked outside a user gesture.
        });
      }
      return true;
    },

    gl_poll_events() {
      return !ctrl.shouldClose;
    },

    gl_swap_buffers() {
      gl.flush();
    },

    gl_clear(color) {
      // Loft rgba() packs as 0xAARRGGBB
      const a = ((color >>> 24) & 0xff) / 255;
      const r = ((color >>> 16) & 0xff) / 255;
      const g = ((color >>> 8)  & 0xff) / 255;
      const b = (color & 0xff) / 255;
      gl.clearColor(r, g, b, a);
      gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
    },

    gl_destroy_window() {
      for (const p of programs) if (p) gl.deleteProgram(p);
      for (const v of vaos) if (v) gl.deleteVertexArray(v.vao);
      for (const t of textures) if (t) gl.deleteTexture(t);
      for (const f of fbos) if (f) gl.deleteFramebuffer(f);
      programs = [];
      vaos = [];
      textures = [];
      fbos = [];
    },

    // Shaders
    gl_create_shader(vertSrc, fragSrc) {
      const vs = gl.createShader(gl.VERTEX_SHADER);
      gl.shaderSource(vs, vertSrc);
      gl.compileShader(vs);
      if (!gl.getShaderParameter(vs, gl.COMPILE_STATUS)) {
        console.error('Vertex shader:', gl.getShaderInfoLog(vs));
        gl.deleteShader(vs);
        return -1;
      }
      const fs = gl.createShader(gl.FRAGMENT_SHADER);
      gl.shaderSource(fs, fragSrc);
      gl.compileShader(fs);
      if (!gl.getShaderParameter(fs, gl.COMPILE_STATUS)) {
        console.error('Fragment shader:', gl.getShaderInfoLog(fs));
        gl.deleteShader(vs);
        gl.deleteShader(fs);
        return -1;
      }
      const prog = gl.createProgram();
      gl.attachShader(prog, vs);
      gl.attachShader(prog, fs);
      gl.linkProgram(prog);
      gl.deleteShader(vs);
      gl.deleteShader(fs);
      if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) {
        console.error('Link:', gl.getProgramInfoLog(prog));
        gl.deleteProgram(prog);
        return -1;
      }
      const idx = programs.length;
      programs.push(prog);
      return idx;
    },

    gl_use_shader(program) {
      if (program >= 0 && program < programs.length) {
        gl.useProgram(programs[program]);
      }
    },

    // Vertex upload + drawing
    gl_upload_vertices(data, stride) {
      const vao = gl.createVertexArray();
      gl.bindVertexArray(vao);
      const vbo = gl.createBuffer();
      gl.bindBuffer(gl.ARRAY_BUFFER, vbo);
      gl.bufferData(gl.ARRAY_BUFFER, data, gl.STATIC_DRAW);
      const bpv = stride * 4;
      gl.enableVertexAttribArray(0);
      gl.vertexAttribPointer(0, 3, gl.FLOAT, false, bpv, 0);
      if (stride >= 6) {
        gl.enableVertexAttribArray(1);
        gl.vertexAttribPointer(1, 3, gl.FLOAT, false, bpv, 12);
      }
      if (stride >= 8) {
        gl.enableVertexAttribArray(2);
        gl.vertexAttribPointer(2, 2, gl.FLOAT, false, bpv, 24);
      }
      if (stride >= 10) {
        gl.enableVertexAttribArray(3);
        gl.vertexAttribPointer(3, 2, gl.FLOAT, false, bpv, 32);
      }
      gl.bindVertexArray(null);
      const idx = vaos.length;
      vaos.push({ vao, vbo, vertexCount: data.length / stride });
      return idx;
    },

    gl_draw(vaoIdx, vertexCount) {
      if (vaoIdx >= 0 && vaoIdx < vaos.length) {
        gl.bindVertexArray(vaos[vaoIdx].vao);
        gl.drawArrays(gl.TRIANGLES, 0, vertexCount);
        gl.bindVertexArray(null);
      }
    },

    gl_draw_mode(vaoIdx, vertexCount, mode) {
      if (vaoIdx >= 0 && vaoIdx < vaos.length) {
        gl.bindVertexArray(vaos[vaoIdx].vao);
        gl.drawArrays(glDrawMode(mode), 0, vertexCount);
        gl.bindVertexArray(null);
      }
    },

    gl_draw_elements(vaoIdx, indexCount, mode) {
      if (vaoIdx >= 0 && vaoIdx < vaos.length) {
        gl.bindVertexArray(vaos[vaoIdx].vao);
        gl.drawElements(glDrawMode(mode), indexCount, gl.UNSIGNED_INT, 0);
        gl.bindVertexArray(null);
      }
    },

    gl_draw_fullscreen_quad() {
      // Lazy-init a fullscreen quad VAO: 2 triangles, pos(xy) + uv(st)
      if (!ctrl._fsQuad) {
        const vao = gl.createVertexArray();
        gl.bindVertexArray(vao);
        const vbo = gl.createBuffer();
        gl.bindBuffer(gl.ARRAY_BUFFER, vbo);
        gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([
          -1, -1, 0, 0,
           1, -1, 1, 0,
          -1,  1, 0, 1,
           1, -1, 1, 0,
           1,  1, 1, 1,
          -1,  1, 0, 1,
        ]), gl.STATIC_DRAW);
        // location 0 = position (vec2), location 1 = UV (vec2)
        gl.enableVertexAttribArray(0);
        gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 16, 0);
        gl.enableVertexAttribArray(1);
        gl.vertexAttribPointer(1, 2, gl.FLOAT, false, 16, 8);
        gl.bindVertexArray(null);
        ctrl._fsQuad = vao;
      }
      gl.bindVertexArray(ctrl._fsQuad);
      gl.drawArrays(gl.TRIANGLES, 0, 6);
      gl.bindVertexArray(null);
    },

    // Uniforms
    gl_set_uniform_mat4(program, name, mat) {
      if (program >= 0 && program < programs.length) {
        const loc = gl.getUniformLocation(programs[program], name);
        if (loc) gl.uniformMatrix4fv(loc, false, mat);
      }
    },

    gl_set_uniform_float(program, name, val) {
      if (program >= 0 && program < programs.length) {
        const loc = gl.getUniformLocation(programs[program], name);
        if (loc) gl.uniform1f(loc, val);
      }
    },

    gl_set_uniform_int(program, name, val) {
      if (program >= 0 && program < programs.length) {
        const loc = gl.getUniformLocation(programs[program], name);
        if (loc) gl.uniform1i(loc, val);
      }
    },

    gl_set_uniform_vec3(program, name, x, y, z) {
      if (program >= 0 && program < programs.length) {
        const loc = gl.getUniformLocation(programs[program], name);
        if (loc) gl.uniform3f(loc, x, y, z);
      }
    },

    // GL state
    gl_enable(cap) { gl.enable(glCap(cap)); },
    gl_disable(cap) { gl.disable(glCap(cap)); },

    gl_blend_func(src, dst) {
      gl.blendFunc(glBlendFactor(src), glBlendFactor(dst));
    },

    gl_cull_face(face) {
      gl.cullFace(face === 1 ? gl.FRONT : gl.BACK);
    },

    gl_depth_mask(write) { gl.depthMask(write); },
    gl_viewport(x, y, w, h) { gl.viewport(x, y, w, h); },
    gl_line_width(width) { gl.lineWidth(width); },
    gl_point_size(_size) { /* WebGL: use gl_PointSize in vertex shader */ },

    // Framebuffers
    gl_create_framebuffer() {
      const fbo = gl.createFramebuffer();
      // Offset by 1: index 0 = default screen framebuffer (matches native GL)
      const idx = fbos.length + 1;
      fbos.push(fbo);
      return idx;
    },

    gl_bind_framebuffer(fboIdx) {
      if (fboIdx <= 0) {
        gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      } else if (fboIdx - 1 < fbos.length) {
        gl.bindFramebuffer(gl.FRAMEBUFFER, fbos[fboIdx - 1]);
      }
    },

    gl_framebuffer_texture(fboIdx, attachment, texIdx) {
      const fi = fboIdx - 1;
      if (fi >= 0 && fi < fbos.length && texIdx >= 0 && texIdx < textures.length) {
        gl.bindFramebuffer(gl.FRAMEBUFFER, fbos[fi]);
        const attach = attachment === 1 ? gl.DEPTH_ATTACHMENT : gl.COLOR_ATTACHMENT0;
        gl.framebufferTexture2D(gl.FRAMEBUFFER, attach, gl.TEXTURE_2D, textures[texIdx], 0);
      }
    },

    gl_create_depth_texture(w, h) {
      const tex = gl.createTexture();
      gl.bindTexture(gl.TEXTURE_2D, tex);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.DEPTH_COMPONENT24, w, h, 0, gl.DEPTH_COMPONENT, gl.UNSIGNED_INT, null);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      const idx = textures.length;
      textures.push(tex);
      return idx;
    },

    gl_create_color_texture(w, h) {
      const tex = gl.createTexture();
      gl.bindTexture(gl.TEXTURE_2D, tex);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      const idx = textures.length;
      textures.push(tex);
      return idx;
    },

    // Textures
    gl_load_texture(path) {
      const asset = ctrl.assets[path];
      if (!asset || !asset.image) return -1;
      const img = asset.image;
      const tex = gl.createTexture();
      gl.bindTexture(gl.TEXTURE_2D, tex);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, img);
      gl.generateMipmap(gl.TEXTURE_2D);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.REPEAT);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.REPEAT);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR_MIPMAP_LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      const idx = textures.length;
      textures.push(tex);
      return idx;
    },

    gl_upload_canvas(data, w, h) {
      // C58: canonical `(0, 0) = canvas top-left` — no upload-side Y flip.
      // glTexImage2D stores the first buffer row at GL bottom, putting
      // canvas-top at TC.y=0; the 2D ortho's `-2/H` maps that to screen-top.
      const pixels = new Uint8Array(w * h * 4);
      for (let y = 0; y < h; y++) {
        for (let x = 0; x < w; x++) {
          const si = y * w + x;
          const di = (y * w + x) * 4;
          const c = data[si];
          pixels[di + 0] = (c >>> 16) & 0xff;
          pixels[di + 1] = (c >>> 8) & 0xff;
          pixels[di + 2] = c & 0xff;
          pixels[di + 3] = (c >>> 24) & 0xff;
        }
      }
      const tex = gl.createTexture();
      gl.bindTexture(gl.TEXTURE_2D, tex);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, pixels);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      const idx = textures.length;
      textures.push(tex);
      return idx;
    },

    gl_bind_texture(texIdx, unit) {
      gl.activeTexture(gl.TEXTURE0 + unit);
      if (texIdx >= 0 && texIdx < textures.length) {
        gl.bindTexture(gl.TEXTURE_2D, textures[texIdx]);
      }
    },

    gl_delete_texture(texIdx) {
      if (texIdx >= 0 && texIdx < textures.length && textures[texIdx]) {
        gl.deleteTexture(textures[texIdx]);
        textures[texIdx] = null;
      }
    },

    // Cleanup
    gl_delete_shader(progIdx) {
      if (progIdx >= 0 && progIdx < programs.length && programs[progIdx]) {
        gl.deleteProgram(programs[progIdx]);
        programs[progIdx] = null;
      }
    },

    gl_delete_vao(vaoIdx) {
      if (vaoIdx >= 0 && vaoIdx < vaos.length && vaos[vaoIdx]) {
        gl.deleteVertexArray(vaos[vaoIdx].vao);
        gl.deleteBuffer(vaos[vaoIdx].vbo);
        vaos[vaoIdx] = null;
      }
    },

    gl_delete_framebuffer(fboIdx) {
      const fi = fboIdx - 1;
      if (fi >= 0 && fi < fbos.length && fbos[fi]) {
        gl.deleteFramebuffer(fbos[fi]);
        fbos[fi] = null;
      }
    },

    // File I/O
    save_png(path, w, h, data) {
      const c2 = document.createElement('canvas');
      c2.width = w; c2.height = h;
      const ctx = c2.getContext('2d');
      const imgData = ctx.createImageData(w, h);
      for (let i = 0; i < data.length; i++) {
        const px = data[i];
        imgData.data[i * 4 + 0] = (px >>> 16) & 0xff;
        imgData.data[i * 4 + 1] = (px >>> 8) & 0xff;
        imgData.data[i * 4 + 2] = px & 0xff;
        imgData.data[i * 4 + 3] = (px >>> 24) & 0xff;
      }
      ctx.putImageData(imgData, 0, 0);
      c2.toBlob(blob => {
        const a = document.createElement('a');
        a.href = URL.createObjectURL(blob);
        a.download = path;
        a.click();
        URL.revokeObjectURL(a.href);
      });
    },

    // Binary asset loading (fonts, etc.) — returns Uint8Array or null.
    load_binary_asset(path) {
      const asset = ctrl.assets[path];
      if (asset && asset.bytes) return asset.bytes;
      for (const [k, v] of Object.entries(ctrl.assets)) {
        if (path.endsWith(k) && v.bytes) return v.bytes;
      }
      return null;
    },

    // Font/text — now handled by fontdue in WASM (Rust-side).
    // These JS stubs remain as fallbacks if fontdue is not available.
    gl_load_font(_path) { return -1; },
    gl_measure_text(_fontIdx, _text, _size) { return 0.0; },
    gl_text_height(_fontIdx, size) { return Math.ceil(size * 1.2); },

    // Input
    gl_key_pressed(keyCode) { return keys.has(keyCode); },
    gl_mouse_x() { return mouseX; },
    gl_mouse_y() { return mouseY; },
    gl_mouse_button() { return mouseBtn; },
    gl_mouse_wheel() {
      const v = wheelAccum;
      wheelAccum = 0;
      return v;
    },
  });

  return ctrl;
}
