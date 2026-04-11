// loft-gl-wasm.js — WebGL2 bridge for compiled loft WASM (--html export).
// Returns a WASM imports object with loft_gl.* and loft_io.* modules.
// Unlike loft-gl.js (which uses loftHost), this reads raw pointers from
// WASM linear memory for string/vector arguments.

function buildLoftImports(canvas, output, getMem) {
  const gl = canvas.getContext('webgl2', { antialias: true, alpha: false });
  const decoder = new TextDecoder();
  function readStr(ptr, len) {
    return decoder.decode(new Uint8Array(getMem().buffer, ptr, len));
  }

  let programs = [], vaos = [], textures = [], fbos = [];
  const keys = new Set();
  let mouseX = 0, mouseY = 0, mouseBtn = 0;

  function mapKey(code) {
    if (code.startsWith('Key')) return code.charCodeAt(3) + 32;
    if (code.startsWith('Digit')) return code.charCodeAt(5);
    const s = { ArrowUp:128, ArrowDown:129, ArrowLeft:130, ArrowRight:131,
      ShiftLeft:132, ShiftRight:132, ControlLeft:133, ControlRight:133,
      Space:32, Enter:13, Escape:27, Tab:9 };
    return s[code] || 0;
  }
  function glCap(c) { return [0, gl.DEPTH_TEST, gl.BLEND, gl.CULL_FACE][c] || c; }
  function glBF(f) { return [gl.ZERO, gl.ONE, gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA, gl.DST_ALPHA, gl.ONE_MINUS_DST_ALPHA][f] || f; }
  function glMode(m) { return [gl.TRIANGLES, gl.LINES, gl.POINTS][m] || gl.TRIANGLES; }

  canvas.tabIndex = 0;
  canvas.addEventListener('keydown', e => { keys.add(mapKey(e.code)); e.preventDefault(); });
  canvas.addEventListener('keyup', e => keys.delete(mapKey(e.code)));
  canvas.addEventListener('mousemove', e => {
    const r = canvas.getBoundingClientRect();
    mouseX = e.clientX - r.left; mouseY = e.clientY - r.top;
  });
  canvas.addEventListener('mousedown', e => { mouseBtn |= (1 << e.button); });
  canvas.addEventListener('mouseup', e => { mouseBtn &= ~(1 << e.button); });

  let shouldClose = false, _fsQuad = null;

  return {
    loft_io: {
      loft_host_print(ptr, len) { output.textContent += readStr(ptr, len); }
    },
    loft_gl: {
      loft_gl_create_window(w, h, tp, tl) {
        canvas.width = w; canvas.height = h;
        canvas.style.display = 'block';
        output.style.display = 'none';
        gl.viewport(0, 0, w, h);
        gl.enable(gl.DEPTH_TEST);
        shouldClose = false;
        return 1;
      },
      loft_gl_poll_events() { return shouldClose ? 0 : 1; },
      loft_gl_swap_buffers() { gl.flush(); },
      loft_gl_clear(color) {
        const a = ((color >>> 24) & 0xff) / 255, r = ((color >>> 16) & 0xff) / 255;
        const g = ((color >>> 8) & 0xff) / 255, b = (color & 0xff) / 255;
        gl.clearColor(r, g, b, a);
        gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
      },
      loft_gl_destroy_window() {
        for (const p of programs) if (p) gl.deleteProgram(p);
        for (const v of vaos) if (v) gl.deleteVertexArray(v.vao);
        for (const t of textures) if (t) gl.deleteTexture(t);
        for (const f of fbos) if (f) gl.deleteFramebuffer(f);
        programs = []; vaos = []; textures = []; fbos = [];
      },
      loft_gl_create_shader(vp, vl, fp, fl) {
        const vertSrc = readStr(vp, vl), fragSrc = readStr(fp, fl);
        const vs = gl.createShader(gl.VERTEX_SHADER);
        gl.shaderSource(vs, vertSrc); gl.compileShader(vs);
        if (!gl.getShaderParameter(vs, gl.COMPILE_STATUS)) {
          console.error('Vertex:', gl.getShaderInfoLog(vs)); gl.deleteShader(vs); return 0;
        }
        const fs = gl.createShader(gl.FRAGMENT_SHADER);
        gl.shaderSource(fs, fragSrc); gl.compileShader(fs);
        if (!gl.getShaderParameter(fs, gl.COMPILE_STATUS)) {
          console.error('Fragment:', gl.getShaderInfoLog(fs)); gl.deleteShader(vs); gl.deleteShader(fs); return 0;
        }
        const p = gl.createProgram();
        gl.attachShader(p, vs); gl.attachShader(p, fs); gl.linkProgram(p);
        gl.deleteShader(vs); gl.deleteShader(fs);
        if (!gl.getProgramParameter(p, gl.LINK_STATUS)) {
          console.error('Link:', gl.getProgramInfoLog(p)); gl.deleteProgram(p); return 0;
        }
        const idx = programs.length; programs.push(p); return idx;
      },
      loft_gl_use_shader(p) { if (p >= 0 && p < programs.length) gl.useProgram(programs[p]); },
      loft_gl_upload_vertices(ptr, count, stride) {
        const data = new Float32Array(getMem().buffer, ptr, count);
        const vao = gl.createVertexArray();
        gl.bindVertexArray(vao);
        const vbo = gl.createBuffer();
        gl.bindBuffer(gl.ARRAY_BUFFER, vbo);
        gl.bufferData(gl.ARRAY_BUFFER, data, gl.STATIC_DRAW);
        const bpv = stride * 4;
        gl.enableVertexAttribArray(0); gl.vertexAttribPointer(0, 3, gl.FLOAT, false, bpv, 0);
        if (stride >= 6) { gl.enableVertexAttribArray(1); gl.vertexAttribPointer(1, 3, gl.FLOAT, false, bpv, 12); }
        if (stride >= 8) { gl.enableVertexAttribArray(2); gl.vertexAttribPointer(2, 2, gl.FLOAT, false, bpv, 24); }
        if (stride >= 10) { gl.enableVertexAttribArray(2); gl.vertexAttribPointer(2, 4, gl.FLOAT, false, bpv, 24); }
        gl.bindVertexArray(null);
        const idx = vaos.length; vaos.push({ vao, vbo, n: count / stride }); return idx;
      },
      loft_gl_draw(vaoIdx, n) {
        if (vaoIdx >= 0 && vaoIdx < vaos.length) { gl.bindVertexArray(vaos[vaoIdx].vao); gl.drawArrays(gl.TRIANGLES, 0, n); gl.bindVertexArray(null); }
      },
      loft_gl_draw_mode(v, n, m) {
        if (v >= 0 && v < vaos.length) { gl.bindVertexArray(vaos[v].vao); gl.drawArrays(glMode(m), 0, n); gl.bindVertexArray(null); }
      },
      loft_gl_draw_elements(v, n, m) {
        if (v >= 0 && v < vaos.length) { gl.bindVertexArray(vaos[v].vao); gl.drawElements(glMode(m), n, gl.UNSIGNED_INT, 0); gl.bindVertexArray(null); }
      },
      loft_gl_draw_fullscreen_quad() {
        if (!_fsQuad) {
          const v = gl.createVertexArray(); gl.bindVertexArray(v);
          const b = gl.createBuffer(); gl.bindBuffer(gl.ARRAY_BUFFER, b);
          gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1,-1,0,0, 1,-1,1,0, -1,1,0,1, 1,-1,1,0, 1,1,1,1, -1,1,0,1]), gl.STATIC_DRAW);
          gl.enableVertexAttribArray(0); gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 16, 0);
          gl.enableVertexAttribArray(1); gl.vertexAttribPointer(1, 2, gl.FLOAT, false, 16, 8);
          gl.bindVertexArray(null); _fsQuad = v;
        }
        gl.bindVertexArray(_fsQuad); gl.drawArrays(gl.TRIANGLES, 0, 6); gl.bindVertexArray(null);
      },
      loft_gl_set_mat4(prog, np, nl, mp, mc) {
        if (prog >= 0 && prog < programs.length) {
          const name = readStr(np, nl);
          const f64 = new Float64Array(getMem().buffer, mp, mc < 16 ? 0 : 16);
          const f32 = new Float32Array(16); for (let i = 0; i < 16; i++) f32[i] = f64[i];
          const loc = gl.getUniformLocation(programs[prog], name);
          if (loc) gl.uniformMatrix4fv(loc, false, f32);
        }
      },
      loft_gl_set_uniform_float(p, np, nl, v) {
        if (p >= 0 && p < programs.length) { const loc = gl.getUniformLocation(programs[p], readStr(np, nl)); if (loc) gl.uniform1f(loc, v); }
      },
      loft_gl_set_uniform_int(p, np, nl, v) {
        if (p >= 0 && p < programs.length) { const loc = gl.getUniformLocation(programs[p], readStr(np, nl)); if (loc) gl.uniform1i(loc, v); }
      },
      loft_gl_set_uniform_vec3(p, np, nl, x, y, z) {
        if (p >= 0 && p < programs.length) { const loc = gl.getUniformLocation(programs[p], readStr(np, nl)); if (loc) gl.uniform3f(loc, x, y, z); }
      },
      loft_gl_enable(c) { gl.enable(glCap(c)); },
      loft_gl_disable(c) { gl.disable(glCap(c)); },
      loft_gl_blend_func(s, d) { gl.blendFunc(glBF(s), glBF(d)); },
      loft_gl_cull_face(f) { gl.cullFace(f === 1 ? gl.FRONT : gl.BACK); },
      loft_gl_depth_mask(w) { gl.depthMask(!!w); },
      loft_gl_viewport(x, y, w, h) { gl.viewport(x, y, w, h); },
      loft_gl_line_width(w) { gl.lineWidth(w); },
      loft_gl_point_size(_s) { /* use gl_PointSize in shader */ },
      loft_gl_create_framebuffer() { const f = gl.createFramebuffer(); const i = fbos.length + 1; fbos.push(f); return i; },
      loft_gl_bind_framebuffer(i) { gl.bindFramebuffer(gl.FRAMEBUFFER, i <= 0 ? null : fbos[i-1] || null); },
      loft_gl_framebuffer_texture(fi, att, ti) {
        if (fi > 0 && fi - 1 < fbos.length && ti >= 0 && ti < textures.length) {
          gl.bindFramebuffer(gl.FRAMEBUFFER, fbos[fi-1]);
          gl.framebufferTexture2D(gl.FRAMEBUFFER, att === 1 ? gl.DEPTH_ATTACHMENT : gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, textures[ti], 0);
        }
      },
      loft_gl_create_depth_texture(w, h) {
        const t = gl.createTexture(); gl.bindTexture(gl.TEXTURE_2D, t);
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.DEPTH_COMPONENT24, w, h, 0, gl.DEPTH_COMPONENT, gl.UNSIGNED_INT, null);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
        const i = textures.length; textures.push(t); return i;
      },
      loft_gl_create_color_texture(w, h) {
        const t = gl.createTexture(); gl.bindTexture(gl.TEXTURE_2D, t);
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
        const i = textures.length; textures.push(t); return i;
      },
      loft_gl_load_texture(pp, pl) { return -1; /* TODO: async asset loading */ },
      loft_gl_upload_canvas(ptr, count, w, h) {
        const data = new Int32Array(getMem().buffer, ptr, count);
        const px = new Uint8Array(w * h * 4);
        for (let y = 0; y < h; y++) { const sy = h - 1 - y;
          for (let x = 0; x < w; x++) { const c = data[sy * w + x], di = (y * w + x) * 4;
            px[di] = (c>>>16)&0xff; px[di+1] = (c>>>8)&0xff; px[di+2] = c&0xff; px[di+3] = (c>>>24)&0xff;
          }
        }
        const t = gl.createTexture(); gl.bindTexture(gl.TEXTURE_2D, t);
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, px);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
        const i = textures.length; textures.push(t); return i;
      },
      loft_gl_bind_texture(ti, u) { gl.activeTexture(gl.TEXTURE0 + u); if (ti >= 0 && ti < textures.length) gl.bindTexture(gl.TEXTURE_2D, textures[ti]); },
      loft_gl_delete_texture(ti) { if (ti >= 0 && ti < textures.length && textures[ti]) { gl.deleteTexture(textures[ti]); textures[ti] = null; } },
      loft_gl_delete_shader(p) { if (p >= 0 && p < programs.length && programs[p]) { gl.deleteProgram(programs[p]); programs[p] = null; } },
      loft_gl_delete_vao(v) { if (v >= 0 && v < vaos.length && vaos[v]) { gl.deleteVertexArray(vaos[v].vao); gl.deleteBuffer(vaos[v].vbo); vaos[v] = null; } },
      loft_gl_delete_framebuffer(fi) { const i = fi-1; if (i >= 0 && i < fbos.length && fbos[i]) { gl.deleteFramebuffer(fbos[i]); fbos[i] = null; } },
      loft_gl_key_pressed(k) { return keys.has(k) ? 1 : 0; },
      loft_gl_mouse_x() { return mouseX; },
      loft_gl_mouse_y() { return mouseY; },
      loft_gl_mouse_button() { return mouseBtn; },
      loft_gl_load_font(pp, pl) { return -2147483648; /* i32::MIN = null sentinel */ },
      loft_gl_measure_text(fi, tp, tl, sz) { return 0.0; },
      loft_text_height(fi, sz) { return Math.ceil(sz * 1.2); },
      loft_rasterize_text_into(fi, tp, tl, sz, bp, bc) { return 0; },
      loft_save_png(pp, pl, w, h, dp, dc) { return 0; },
      loft_gl_upload_alpha_texture(dp, w, h) { return 0; },
      loft_gl_text_texture(fi, tp, tl, sz, wp, hp) { return 0; },
    },
    env: {}
  };
}
