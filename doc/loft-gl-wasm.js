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
  let mouseX = 0, mouseY = 0, mouseBtn = 0, wheelAcc = 0;

  // Handles are 1-BASED here, so 0 means failure and nothing else — the contract
  // `lib/graphics` documents ("0 on failure") and what the native backend gives you,
  // since a real GL object name is never 0.  These tables used to hand back the array
  // index, so the FIRST shader, VAO or texture a page created came back as 0: the
  // documented `if (prog == 0) { fail }` rejected working code, and a client using 0 as
  // its "slot is free" sentinel lost the first mesh it uploaded and then handed that slot
  // to the next one (loft#669).  `fbos` was already 1-based; these three now match.
  const hold = (arr, obj) => arr.push(obj);          // push returns the new length = handle
  const slot = (arr, h) => (h > 0 && h <= arr.length ? arr[h - 1] : null);

  function mapKey(code) {
    if (code.startsWith('Key')) return code.charCodeAt(3) + 32;
    if (code.startsWith('Digit')) return code.charCodeAt(5);
    const s = { ArrowUp:128, ArrowDown:129, ArrowLeft:130, ArrowRight:131,
      ShiftLeft:132, ShiftRight:132, ControlLeft:133, ControlRight:133,
      Space:32, Enter:13, Escape:27, Tab:9 };
    return s[code] || 0;
  }
  // ── text (loft#737) ────────────────────────────────────────────────────────
  // The browser already HAS a rasteriser, so the text bridge is a 2D canvas:
  // `measureText` for the metrics and `fillText` for the coverage bitmap.  The
  // desktop backend uses fontdue and returns an 8-bit alpha bitmap whose height is
  // the line height and whose baseline sits `ascent` from the top; drawing white on
  // transparent reproduces exactly that — the alpha channel IS the coverage.
  //
  // A font PATH resolves to a CSS family rather than a file: there is no synchronous
  // way to load font bytes here, and an async load would change the metrics between
  // the measure and the rasterise of the same string.  A page that wants its real
  // font declares it with `@font-face` under the file's base name (in the `[wasm.bridge]
  // host_js` or the page's own CSS); `document.fonts.check` then finds it and it is
  // used exactly. Otherwise the base name picks a generic family, so text still draws.
  let fonts = [], textCv = null, textCx = null;
  function text2d() {
    if (!textCx) {
      textCv = document.createElement('canvas');
      textCx = textCv.getContext('2d', { willReadFrequently: true });
    }
    return textCx;
  }
  function familyFor(base) {
    const quoted = '"' + base.replace(/"/g, '') + '"';
    try {
      // A family the page registered itself wins — this is the exact-font path.
      if (document.fonts && document.fonts.check('16px ' + quoted)) return quoted + ', sans-serif';
    } catch (e) { /* check() throws on a malformed family — fall through */ }
    const b = base.toLowerCase();
    if (/mono|courier|consol|code/.test(b)) return 'monospace';
    if (/serif/.test(b) && !/sans/.test(b)) return 'serif';
    return 'sans-serif';
  }
  // Metrics for font `fi` at `sz`, in the same terms the desktop backend reports:
  // `asc`/`desc` from the font's own box, `line` the height a bitmap gets.
  function fontMetrics(fi, sz) {
    const f = fonts[fi];
    const cx = text2d();
    cx.font = sz + 'px ' + (f ? f.family : 'sans-serif');
    const m = cx.measureText('Mg');
    const asc = m.fontBoundingBoxAscent || m.actualBoundingBoxAscent || sz * 0.8;
    const desc = m.fontBoundingBoxDescent || m.actualBoundingBoxDescent || sz * 0.2;
    return { cx: cx, asc: asc, line: Math.max(1, Math.ceil(asc + desc)) };
  }
  // Draw `s` white-on-transparent and hand back its RGBA pixels + size, or null when
  // there is nothing to draw.  The single place the bitmap's geometry is decided, so
  // `rasterize_text_into`, `text_texture` and `measure_text` cannot disagree about it.
  function rasterText(fi, s, sz) {
    if (!fonts[fi] || !s) return null;
    const mt = fontMetrics(fi, sz);
    const w = Math.ceil(mt.cx.measureText(s).width);
    const h = mt.line;
    if (w <= 0 || h <= 0) return null;
    textCv.width = w; textCv.height = h;
    // Resizing the canvas resets its context state, so re-apply the font.
    mt.cx.font = sz + 'px ' + fonts[fi].family;
    mt.cx.clearRect(0, 0, w, h);
    mt.cx.fillStyle = '#fff';
    mt.cx.textBaseline = 'alphabetic';
    mt.cx.fillText(s, 0, mt.asc);
    return { w: w, h: h, px: mt.cx.getImageData(0, 0, w, h).data };
  }
  // An 8-bit coverage bitmap as a GL texture. WebGL2 has no TEXTURE_SWIZZLE, which is
  // how the desktop backend makes a RED texture sample as (1,1,1,r), so expand to RGBA
  // here — same sampling result, and the shaders stay identical across targets.
  function alphaTexture(alphaAt, w, h) {
    const px = new Uint8Array(w * h * 4);
    for (let i = 0; i < w * h; i++) {
      px[i * 4] = 255; px[i * 4 + 1] = 255; px[i * 4 + 2] = 255; px[i * 4 + 3] = alphaAt(i);
    }
    const t = gl.createTexture(); gl.bindTexture(gl.TEXTURE_2D, t);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, px);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    return hold(textures, t);
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
  // `gl_mouse_wheel` is documented as the delta accumulated since the last call,
  // positive = UP, with pixel-delta (trackpad) scrolling quantised at 20 px/line.
  // The browser's deltaY is positive DOWNWARD, hence the negation.
  canvas.addEventListener('wheel', e => {
    wheelAcc += e.deltaMode === 0 ? -e.deltaY / 20 : -e.deltaY;
    e.preventDefault();
  }, { passive: false });

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
      // loft#851 — the page's filesystem, from the loft-fs.js blob the HTML
      // template inlines just above this one.  Spread FIRST so a handler below
      // could deliberately override one; nothing does today.
      ...(typeof loftFSImports === 'function' ? loftFSImports(getMem) : {}),
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
      // loft#678 — the RANGE arm of the same bridge, behind the working-set store
      // loaders (store_load_key / store_load_keys / store_load_key_text /
      // store_load_range): one `Range: bytes=off-(off+len-1)` GET instead of a whole
      // file, so a page reads the few pages a lookup touches out of a large hosted
      // store. Same two-phase asyncify shape as loft_host_http_get above, and it
      // shares the one response stash — safe because a suspend bridges a SYNCHRONOUS
      // loft call, so a second request cannot start before this one has rewound.
      // off/len arrive as plain numbers (the import declares f64: exact below 2^53).
      loft_host_http_range(ptr, len, off, n) {
        const ac = asyncCtrl && asyncCtrl.ac;
        if (ac && ac.exports.asyncify_get_state() === 2) {
          ac.suspend();
          return asyncCtrl.httpBytes ? asyncCtrl.httpBytes.length : 0xffffffff;
        }
        if (!ac) return 0xffffffff;  // no asyncify driver -> fetch unavailable
        const url = readStr(ptr, len);
        asyncCtrl.httpBytes = null;
        asyncCtrl.httpTotal = -1;
        const last = off + n - 1;
        fetch(url, { headers: { Range: `bytes=${off}-${last}` } })
          .then(async r => {
            // The total comes from Content-Range (`bytes a-b/TOTAL`) so size() needs
            // no second round trip; Content-Length is the fallback for a 200.
            const cr = r.headers.get('Content-Range');
            if (cr) { const t = cr.split('/').pop(); asyncCtrl.httpTotal = (t && t !== '*') ? Number(t) : -1; }
            else { const cl = r.headers.get('Content-Length'); asyncCtrl.httpTotal = cl ? Number(cl) : -1; }
            if (!r.ok) { asyncCtrl.httpBytes = null; }
            else {
              // 206 = the body IS the range. 200 = the server ignored Range and sent
              // the whole file; slice the window so the answer is right either way.
              const b = new Uint8Array(await r.arrayBuffer());
              asyncCtrl.httpBytes = (r.status === 206) ? b : b.subarray(off, off + n);
            }
            ac.resume('loft_start');
          })
          .catch(() => { asyncCtrl.httpBytes = null; ac.resume('loft_start'); });
        ac.suspend();
        return 0;
      },
      loft_host_http_range_total() {
        return asyncCtrl && asyncCtrl.httpTotal != null ? asyncCtrl.httpTotal : -1;
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
        return hold(programs, p);
      },
      loft_gl_use_shader(p) { const o = slot(programs, p); if (o) gl.useProgram(o); },
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
        return hold(vaos, { vao, vbo, n: count / stride });
      },
      loft_gl_draw(vaoIdx, n) {
        const o = slot(vaos, vaoIdx); if (o) { gl.bindVertexArray(o.vao); gl.drawArrays(gl.TRIANGLES, 0, n); gl.bindVertexArray(null); }
      },
      loft_gl_draw_mode(v, n, m) {
        const o = slot(vaos, v); if (o) { gl.bindVertexArray(o.vao); gl.drawArrays(glMode(m), 0, n); gl.bindVertexArray(null); }
      },
      loft_gl_draw_elements(v, n, m) {
        const o = slot(vaos, v); if (o) { gl.bindVertexArray(o.vao); gl.drawElements(glMode(m), n, gl.UNSIGNED_INT, 0); gl.bindVertexArray(null); }
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
        const o = slot(programs, prog);
        if (o) {
          const name = readStr(np, nl);
          const f64 = new Float64Array(getMem().buffer, mp, mc < 16 ? 0 : 16);
          const f32 = new Float32Array(16); for (let i = 0; i < 16; i++) f32[i] = f64[i];
          const loc = gl.getUniformLocation(o, name);
          if (loc) gl.uniformMatrix4fv(loc, false, f32);
        }
      },
      loft_gl_set_uniform_float(p, np, nl, v) {
        const o = slot(programs, p); if (o) { const loc = gl.getUniformLocation(o, readStr(np, nl)); if (loc) gl.uniform1f(loc, v); }
      },
      loft_gl_set_uniform_int(p, np, nl, v) {
        const o = slot(programs, p); if (o) { const loc = gl.getUniformLocation(o, readStr(np, nl)); if (loc) gl.uniform1i(loc, v); }
      },
      loft_gl_set_uniform_vec3(p, np, nl, x, y, z) {
        const o = slot(programs, p); if (o) { const loc = gl.getUniformLocation(o, readStr(np, nl)); if (loc) gl.uniform3f(loc, x, y, z); }
      },
      loft_gl_enable(c) { gl.enable(glCap(c)); },
      loft_gl_disable(c) { gl.disable(glCap(c)); },
      loft_gl_blend_func(s, d) { gl.blendFunc(glBF(s), glBF(d)); },
      loft_gl_cull_face(f) { gl.cullFace(f === 1 ? gl.FRONT : gl.BACK); },
      loft_gl_depth_mask(w) { gl.depthMask(!!w); },
      loft_gl_viewport(x, y, w, h) { gl.viewport(x, y, w, h); },
      loft_gl_line_width(w) { gl.lineWidth(w); },
      loft_gl_point_size(_s) { /* use gl_PointSize in shader */ },
      loft_gl_create_framebuffer() { return hold(fbos, gl.createFramebuffer()); },
      loft_gl_bind_framebuffer(i) { gl.bindFramebuffer(gl.FRAMEBUFFER, slot(fbos, i)); },
      loft_gl_framebuffer_texture(fi, att, ti) {
        const fb = slot(fbos, fi), tex = slot(textures, ti);
        if (fb && tex) {
          gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
          gl.framebufferTexture2D(gl.FRAMEBUFFER, att === 1 ? gl.DEPTH_ATTACHMENT : gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0);
        }
      },
      loft_gl_create_depth_texture(w, h) {
        const t = gl.createTexture(); gl.bindTexture(gl.TEXTURE_2D, t);
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.DEPTH_COMPONENT24, w, h, 0, gl.DEPTH_COMPONENT, gl.UNSIGNED_INT, null);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
        return hold(textures, t);
      },
      loft_gl_create_color_texture(w, h) {
        const t = gl.createTexture(); gl.bindTexture(gl.TEXTURE_2D, t);
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, null);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
        return hold(textures, t);
      },
      // 0 = failure, the same sentinel every other handle here uses (loft#669) — and
      // never a valid handle, since `hold` is 1-based, so a caller CAN tell the two
      // apart.  loft#738: a BUNDLED asset works here. `--html` embeds every `.png`
      // sibling of the entry file and `decodeLoftAssets` decodes them to raw pixels
      // BEFORE `loft_start`, so the lookup is synchronous and no fetch is involved.
      // Only a path with no bundled asset — a runtime URL — is still unsupported;
      // that one genuinely needs async, and it reports failure rather than pretending.
      loft_gl_load_texture(pp, pl) {
        const name = readStr(pp, pl).split(/[\\/]/).pop();
        const a = (ctrl && ctrl.assets) ? ctrl.assets[name] : null;
        if (!a || !a.bytes || a.width <= 0 || a.height <= 0) return 0;
        // The asset table holds RGB; GL wants RGBA. C58: no upload-side Y flip.
        const n = a.width * a.height;
        const px = new Uint8Array(n * 4);
        for (let i = 0; i < n; i++) {
          px[i * 4] = a.bytes[i * 3]; px[i * 4 + 1] = a.bytes[i * 3 + 1];
          px[i * 4 + 2] = a.bytes[i * 3 + 2]; px[i * 4 + 3] = 255;
        }
        const t = gl.createTexture(); gl.bindTexture(gl.TEXTURE_2D, t);
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, a.width, a.height, 0, gl.RGBA, gl.UNSIGNED_BYTE, px);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
        return hold(textures, t);
      },
      loft_gl_upload_canvas(ptr, count, w, h) {
        // C58: no upload-side Y flip; canvas-top = GL TC.y=0.
        //
        // A `vector<integer>` marshals to this import as `*const i64` — codegen picks the
        // element width from the vector's STORAGE STRIDE (`vector_elem_rust_type`, @P310),
        // and a plain `integer` vector strides 8 bytes with the packed 0xAARRGGBB colour in
        // the LOW half.  Reading it at i32 stride took every other 4-byte word, so half the
        // "pixels" were a neighbour's zero high-half — transparent black.  The native
        // backend documents fixing exactly this at the 2c migration ("moiré on textured
        // surfaces, missing pixels on rasterised text"); this bridge still had the old read.
        // `count` is the ELEMENT count, so the word view is twice as long.
        const data = new Int32Array(getMem().buffer, ptr, count * 2);
        const px = new Uint8Array(w * h * 4);
        for (let y = 0; y < h; y++) {
          for (let x = 0; x < w; x++) { const c = data[(y * w + x) * 2], di = (y * w + x) * 4;
            px[di] = (c>>>16)&0xff; px[di+1] = (c>>>8)&0xff; px[di+2] = c&0xff; px[di+3] = (c>>>24)&0xff;
          }
        }
        const t = gl.createTexture(); gl.bindTexture(gl.TEXTURE_2D, t);
        gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, w, h, 0, gl.RGBA, gl.UNSIGNED_BYTE, px);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
        gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
        return hold(textures, t);
      },
      loft_gl_bind_texture(ti, u) { gl.activeTexture(gl.TEXTURE0 + u); const o = slot(textures, ti); if (o) gl.bindTexture(gl.TEXTURE_2D, o); },
      loft_gl_delete_texture(ti) { const o = slot(textures, ti); if (o) { gl.deleteTexture(o); textures[ti - 1] = null; } },
      loft_gl_delete_shader(p) { const o = slot(programs, p); if (o) { gl.deleteProgram(o); programs[p - 1] = null; } },
      loft_gl_delete_vao(v) { const o = slot(vaos, v); if (o) { gl.deleteVertexArray(o.vao); gl.deleteBuffer(o.vbo); vaos[v - 1] = null; } },
      loft_gl_delete_framebuffer(fi) { const o = slot(fbos, fi); if (o) { gl.deleteFramebuffer(o); fbos[fi - 1] = null; } },
      loft_gl_key_pressed(k) { return keys.has(k) ? 1 : 0; },
      loft_gl_mouse_x() { return mouseX; },
      loft_gl_mouse_y() { return mouseY; },
      loft_gl_mouse_button() { return mouseBtn; },
      // The canvas IS the window here, so its size is the window's inner size in
      // physical pixels — exactly what these are documented to report.  They used to be
      // absent, and an absent import is not a missing feature but a dead page: the whole
      // module fails to instantiate (loft#668).
      loft_gl_window_width() { return canvas.width; },
      loft_gl_window_height() { return canvas.height; },
      // Delta since the last call, reset on read.
      loft_gl_mouse_wheel() { const w = Math.trunc(wheelAcc); wheelAcc -= w; return w; },
      // `lib/graphics` already documents this as non-fatal outside a user gesture in the
      // browser, which is exactly what a rejected requestFullscreen() promise is.
      loft_gl_set_fullscreen(on) {
        try {
          if (on) { const p = canvas.requestFullscreen?.(); if (p) p.catch(() => {}); }
          else if (document.fullscreenElement) { const p = document.exitFullscreen?.(); if (p) p.catch(() => {}); }
        } catch (e) { /* no gesture / not permitted — non-fatal by contract */ }
      },
      // loft#737 — the text bridge. A font PATH becomes a CSS family (see `familyFor`);
      // the handle is a 0-BASED index, matching the desktop backend, so 0 is a valid
      // font and the null sentinel is i32::MIN.
      loft_gl_load_font(pp, pl) {
        const path = readStr(pp, pl);
        if (!path) return -2147483648;
        const base = path.split(/[\\/]/).pop().replace(/\.[^.]*$/, '');
        fonts.push({ family: familyFor(base), base: base });
        return fonts.length - 1;
      },
      loft_gl_measure_text(fi, tp, tl, sz) {
        if (!fonts[fi]) return 0.0;
        return fontMetrics(fi, sz).cx.measureText(readStr(tp, tl)).width;
      },
      // The line height a rasterised bitmap gets. The `sz * 1.2` fallback is what the
      // desktop backend also falls back to when a font exposes no usable metrics.
      loft_text_height(fi, sz) {
        if (!fonts[fi]) return Math.ceil(sz * 1.2);
        return fontMetrics(fi, sz).line;
      },
      // @P340 — baseline → top of glyphs, so callers can baseline-align mixed sizes.
      loft_gl_font_ascent(fi, sz) {
        if (!fonts[fi]) return sz * 0.8;
        return fontMetrics(fi, sz).asc;
      },
      // Writes alpha (0-255) into a loft `vector<integer>` and returns the bitmap
      // WIDTH — the caller sized its buffer from `measure_text` + `text_height` and
      // indexes rows by this width. `integer` is i64 storage, so each element is two
      // 32-bit words (little-endian: low word first, high word 0 for 0-255).
      loft_rasterize_text_into(fi, tp, tl, sz, bp, bc) {
        const r = rasterText(fi, readStr(tp, tl), sz);
        if (!r) return 0;
        const words = new Int32Array(getMem().buffer, bp, bc * 2);
        const n = Math.min(r.w * r.h, bc);
        for (let i = 0; i < n; i++) {
          words[i * 2] = r.px[i * 4 + 3];
          words[i * 2 + 1] = 0;
        }
        return r.w;
      },
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
      // loft#738 — an 8-bit coverage buffer the PROGRAM computed, uploaded as a
      // texture. The data is already in wasm memory, so this needs no fetch and no
      // asset pipeline; it is the route for any CPU-rasterised overlay (a glyph atlas,
      // a mask, a generated ramp) and the one that was missing entirely.
      loft_gl_upload_alpha_texture(dp, w, h) {
        if (w <= 0 || h <= 0) return 0;
        const a = new Uint8Array(getMem().buffer, dp, w * h);
        return alphaTexture(i => a[i], w, h);
      },
      // Rasterise + upload in one step, reporting the size through the two out-params
      // (i32, as the desktop backend writes them).
      loft_gl_text_texture(fi, tp, tl, sz, wp, hp) {
        const r = rasterText(fi, readStr(tp, tl), sz);
        const out = new Int32Array(getMem().buffer);
        if (!r) {
          if (wp) out[wp >> 2] = 0;
          if (hp) out[hp >> 2] = 0;
          return 0;
        }
        if (wp) out[wp >> 2] = r.w;
        if (hp) out[hp >> 2] = r.h;
        return alphaTexture(i => r.px[i * 4 + 3], r.w, r.h);
      },
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
