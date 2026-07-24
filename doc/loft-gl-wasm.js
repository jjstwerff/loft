// loft-gl-wasm.js — WebGL2 bridge for compiled loft WASM (--html export).
// Returns a WASM imports object with loft_gl.* and loft_io.* modules.
// Uses asyncify for frame yield: gl_swap_buffers suspends execution so
// the browser can render via requestAnimationFrame.

// Asyncify controller — manages suspend/resume across frames.
// Usage: const ac = new AsyncifyCtrl(instance);
//        ac.start('loft_start');  // runs until first swap_buffers
//        // on each rAF: ac.resume('loft_start');
// @P321c Phase 3b — decode base64-embedded PNG assets to raw RGB bytes.
// `rawAssets` is `{name: base64String, ...}` (Phase 3a embed in main.rs);
// resolves to `{name: {width, height, bytes: Uint8Array(rgb)}}` for the
// imaging bridge to look up sync.  Runs once after `WebAssembly.instantiate`
// + before `loft_start` so the wasm-side imaging_query/copy is synchronous.
async function decodeLoftAssets(rawAssets) {
  if (!rawAssets || typeof rawAssets !== 'object') return {};
  const out = {};
  for (const [name, b64] of Object.entries(rawAssets)) {
    if (typeof b64 !== 'string') { out[name] = b64; continue; }
    try {
      const bin = Uint8Array.from(atob(b64), c => c.charCodeAt(0));
      const blob = new Blob([bin], { type: 'image/png' });
      const bitmap = await createImageBitmap(blob);
      const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
      const ctx = canvas.getContext('2d');
      ctx.drawImage(bitmap, 0, 0);
      const imgData = ctx.getImageData(0, 0, bitmap.width, bitmap.height);
      const rgba = imgData.data;
      const rgb = new Uint8Array(bitmap.width * bitmap.height * 3);
      for (let i = 0, j = 0; i < rgba.length; i += 4, j += 3) {
        rgb[j] = rgba[i];
        rgb[j + 1] = rgba[i + 1];
        rgb[j + 2] = rgba[i + 2];
      }
      out[name] = { width: bitmap.width, height: bitmap.height, bytes: rgb };
    } catch (e) {
      // Leave the entry undecoded; bridge will treat as missing.
      out[name] = null;
    }
  }
  return out;
}

// AsyncifyCtrl moved to doc/loft-asyncify.js (shared with the headless template);
// the HTML page includes that blob before this one.

function buildLoftImports(canvas, output, getMem, asyncCtrl) {
  const gl = canvas.getContext('webgl2', { antialias: true, alpha: false });
  // @PLN117: loftTextDecoder also reads a threaded page's SHARED memory, which
  // a plain TextDecoder refuses; falls back when the page has no threading glue.
  const decoder = typeof loftTextDecoder === 'function' ? loftTextDecoder() : new TextDecoder();
  function readStr(ptr, len) {
    return decoder.decode(new Uint8Array(getMem().buffer, ptr, len));
  }
  // lib/graphics shaders are written for desktop GLSL (#version 330 core);
  // WebGL2 needs GLSL ES 3.00.  The grammar is otherwise compatible for the
  // subset our shaders use (in/out/layout/texture()/discard/gl_Position),
  // so rewrite the version directive + inject default precision and let
  // WebGL2 take everything else as-is.  Fragment shaders need explicit
  // precision for float/int; vertex shaders default to highp.
  function translateShader(src, isFragment) {
    const re = /^\s*#version\s+\d+(\s+\w+)?\s*\n?/;
    const head = isFragment
      ? '#version 300 es\nprecision highp float;\nprecision highp int;\n'
      : '#version 300 es\n';
    if (re.test(src)) return src.replace(re, head);
    return head + src;
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

  // Defensive Number coercion at the WASM-import boundary.  loft's
  // `integer` is i64 since the phase-2c migration; wasm-bindgen's
  // automatic i64 → JsValue path produces BigInt, which silently
  // mis-behaves under the `>>>` / `&` bitwise ops the bridge uses
  // (unpacking RGBA, dispatching enums).  Coerce to Number at entry
  // so every method body can treat args as plain numbers.  Strings
  // pass through (typeof 'foo' !== 'bigint').
  const n = (x) => (typeof x === 'bigint' ? Number(x) : x);
  function coerceArgs(fns) {
    const wrapped = {};
    for (const [name, f] of Object.entries(fns)) {
      wrapped[name] = function(...args) {
        for (let i = 0; i < args.length; i++) args[i] = n(args[i]);
        return f.apply(this, args);
      };
    }
    return wrapped;
  }

  return {
    loft_io: coerceArgs({
      loft_host_print(ptr, len) { output.textContent += readStr(ptr, len); },
      // #620: the browser CLOCK bridge.  wasm32-unknown-unknown has no std
      // clock, so without these now()/ticks() returned a hardcoded 0 and every
      // frame timer read the same instant.  performance.now() is monotonic and
      // page-relative — exactly ticks()'s contract.
      loft_host_time_now_ms() { return Date.now(); },
      loft_host_time_ticks_us() { return performance.now() * 1000; },
      // JS -> loft input is a QUEUE, the mirror of loft_host_print: seed it
      // with globalThis.loftInput (a string) before loft_start, push live
      // messages any time with globalThis.loftPush(msg) — e.g. fetch()
      // completions.  Each host_input() call pops one message via a
      // len+copy pair (loft sizes the buffer, the copy pops).
      loft_host_input_len() {
        if (!globalThis.__loftInQ) {
          globalThis.__loftInQ = [];
          if (globalThis.loftInput != null)
            globalThis.__loftInQ.push(new TextEncoder().encode(String(globalThis.loftInput)));
          globalThis.loftPush = (m) =>
            globalThis.__loftInQ.push(new TextEncoder().encode(String(m)));
        }
        const q = globalThis.__loftInQ;
        return q.length ? q[0].length : 0;
      },
      loft_host_input_copy(ptr) {
        const b = (globalThis.__loftInQ || []).shift();
        if (b) new Uint8Array(getMem().buffer, ptr, b.length).set(b);
      },
      // loft -> JS structured messages (host_output()): the page handles
      // them in globalThis.loftOutput(msg) — the request/response pattern is
      // host_output a request, act on it in JS, loftPush the completion.
      loft_host_output(ptr, len) {
        const m = readStr(ptr, len);
        if (globalThis.loftOutput) globalThis.loftOutput(m);
        else console.log("[loft:out]", m);
      },
      // @PLN97 store_load_url_trusted: async fetch() bridged to a SYNCHRONOUS
      // loft call via asyncify — the same driver the headless page uses (see
      // main.rs / WASM_STORE_LOAD_URL.md).  Invoked TWICE per fetch:
      //  (1) NORMAL state: start fetch(url), then ac.suspend() unwinds the
      //      whole wasm stack to the event loop (return value ignored).
      //  (2) REWINDING (===2): the fetch resolved + resume() replayed the
      //      stack here — ac.suspend() stop_rewinds and we RETURN the byte
      //      length (0xFFFFFFFF on error → net::fetch_bytes maps it to Err).
      // The bytes are copied out separately by loft_host_http_get_copy.
      loft_host_http_get(ptr, len) {
        const ac = asyncCtrl && asyncCtrl.ac;
        if (ac && ac.exports.asyncify_get_state() === 2) {
          ac.suspend();
          return asyncCtrl.httpBytes ? asyncCtrl.httpBytes.length : 0xffffffff;
        }
        if (!ac) return 0xffffffff;  // no asyncify driver -> fetch unavailable
        const url = readStr(ptr, len);
        asyncCtrl.httpBytes = null;
        fetch(url)
          .then(async r => { asyncCtrl.httpBytes = r.ok ? new Uint8Array(await r.arrayBuffer()) : null; ac.resume('loft_start'); })
          .catch(() => { asyncCtrl.httpBytes = null; ac.resume('loft_start'); });
        ac.suspend();
        return 0;
      },
      loft_host_http_get_copy(ptr) {
        if (asyncCtrl && asyncCtrl.httpBytes)
          new Uint8Array(getMem().buffer, ptr, asyncCtrl.httpBytes.length).set(asyncCtrl.httpBytes);
      },
      // @PLN105 Phase 2 — deliver(tag, value): the JS host receives the value's store base +
      // DbRef (store_base, rec, pos) + its layout descriptor (JSON), and reconstructs the value
      // with no serialization via readLoftValue (embedded reader — main.rs inlines
      // doc/loft-deliver.js AHEAD of this file). SYNCHRONOUS — read within this call, the borrow
      // ends on return (§5). A page observes deliveries by setting globalThis.loftDeliver(tag,
      // value, type_id).
      loft_host_deliver(tag, store_base, rec, pos, type_id, dptr, dlen) {
        const desc = JSON.parse(readStr(dptr, dlen));
        const value = readLoftValue(getMem(), store_base, desc, type_id, rec, pos);
        if (globalThis.loftDeliver) globalThis.loftDeliver(tag, value, type_id);
        else console.log("[loft:deliver]", tag, value);
      },
      // @PLN105 Phase 3 — expose/release: a long-lived deliver. Stash a RE-READER closure by tag
      // (loft pins the value's store so its addresses stay valid across frames); a page calls
      // globalThis.loftExposed.get(String(tag))() each frame for a fresh value (re-derives the view
      // from getMem().buffer — memory.grow-safe). release drops the stash.
      loft_host_expose(tag, store_base, rec, pos, type_id, dptr, dlen) {
        const desc = JSON.parse(readStr(dptr, dlen));
        const reread = () => readLoftValue(getMem(), store_base, desc, type_id, rec, pos);
        (globalThis.loftExposed || (globalThis.loftExposed = new Map())).set(String(tag), reread);
        if (globalThis.loftExpose) globalThis.loftExpose(tag, reread, type_id);
      },
      loft_host_release(tag) {
        if (globalThis.loftExposed) globalThis.loftExposed.delete(String(tag));
        if (globalThis.loftRelease) globalThis.loftRelease(tag);
      }
    }),
    loft_gl: coerceArgs({
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
      loft_gl_swap_buffers() {
        gl.flush();
        // Trigger asyncify suspension so the browser can render this frame.
        if (asyncCtrl && asyncCtrl.ac) asyncCtrl.ac.suspend();
      },
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
        const vertSrc = translateShader(readStr(vp, vl), false);
        const fragSrc = translateShader(readStr(fp, fl), true);
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
        // C58: no upload-side Y flip; canvas-top = GL TC.y=0.
        const data = new Int32Array(getMem().buffer, ptr, count);
        const px = new Uint8Array(w * h * 4);
        for (let y = 0; y < h; y++) {
          for (let x = 0; x < w; x++) { const c = data[y * w + x], di = (y * w + x) * 4;
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
      // @lib_plan-29 W1d — generic asset-table existence check; used
      // by `database::io::get_file` so file().png() (and any future
      // asset-using library) sees auto-discovered PNGs as TextFile
      // instead of NotExists.  Library-specific imaging fns
      // (imaging_query / imaging_copy_rgb / imaging_save) live in
      // their package's own host.js — see lib/imaging/wasm/host.js,
      // concatenated into the HTML preamble by `--html` via the
      // `[wasm.bridge].host_js` manifest key.
      host_asset_exists(pp, pl) {
        const name = readStr(pp, pl).split(/[\\/]/).pop();
        return (ctrl.assets && ctrl.assets[name]) ? 1 : 0;
      },
      loft_gl_upload_alpha_texture(dp, w, h) { return 0; },
      loft_gl_text_texture(fi, tp, tl, sz, wp, hp) { return 0; },
      // G5: Audio via Web Audio API
      loft_audio_load(pp, pl) {
        return -2147483648; // i32::MIN — file-based audio not yet supported in WASM
      },
      loft_audio_play(clip, volume) { return -1; },
      loft_audio_stop(sink) {},
      loft_audio_set_volume(sink, volume) {},
      loft_audio_play_raw(ptr, count, sample_rate, volume) {
        try {
          if (!window._loftAudioCtx) window._loftAudioCtx = new AudioContext();
          const ctx = window._loftAudioCtx;
          const f32 = new Float32Array(getMem().buffer, ptr, count);
          const buf = ctx.createBuffer(1, count, sample_rate);
          buf.getChannelData(0).set(f32);
          const src = ctx.createBufferSource();
          src.buffer = buf;
          const gain = ctx.createGain();
          gain.gain.value = volume;
          src.connect(gain);
          gain.connect(ctx.destination);
          src.start();
          return 0;
        } catch(e) { return -1; }
      },
    }),   // ← close coerceArgs() for loft_gl
    env: {}
  };
}
