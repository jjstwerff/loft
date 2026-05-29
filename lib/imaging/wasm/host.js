// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @lib_plan-29 W2 — `lib/imaging`'s JS-side host imports for the
// `--html` build.  Concatenated into the generated HTML preamble by
// the `--html` driver (which reads `[wasm.bridge].host_js = "wasm/
// host.js"` from `lib/imaging/loft.toml`).
//
// Self-registers via a global `LOFT_WASM_EXTENSIONS` array: the
// generic `buildLoftImports` callback chain (defined in
// `doc/loft-gl-wasm.js`) walks this array after constructing the
// imports object and applies each extension to it.  Order of
// extensions matches manifest scan order.
//
// The Rust bridge in `lib/imaging/wasm/src/lib.rs` calls these in
// two steps: imaging_query for dimensions, then imaging_copy_rgb to
// fill a pre-allocated vector payload directly.  imaging_save uses
// OffscreenCanvas.convertToBlob + a synthetic <a download> click.

(globalThis.LOFT_WASM_EXTENSIONS = globalThis.LOFT_WASM_EXTENSIONS || []).push(
  function loftImagingHostImports(imports, ctrl, getMem) {
    const decoder = new TextDecoder();
    function readStr(ptr, len) {
      return decoder.decode(new Uint8Array(getMem().buffer, ptr, len));
    }
    function basename(p) {
      return p.split(/[\\/]/).pop();
    }
    Object.assign(imports.loft_gl, {
      imaging_query(pp, pl, w_out, h_out) {
        const name = basename(readStr(pp, pl));
        const a = ctrl.assets && ctrl.assets[name];
        if (!a || !a.bytes) return 0;
        const mem32 = new Uint32Array(getMem().buffer);
        mem32[w_out >>> 2] = a.width;
        mem32[h_out >>> 2] = a.height;
        return 1;
      },
      imaging_copy_rgb(pp, pl, dest, dest_len) {
        const name = basename(readStr(pp, pl));
        const a = ctrl.assets && ctrl.assets[name];
        if (!a || !a.bytes || a.bytes.length > dest_len) return 0;
        new Uint8Array(getMem().buffer, dest, a.bytes.length).set(a.bytes);
        return 1;
      },
      imaging_save(pp, pl, w, h, dp, dl) {
        try {
          const name = basename(readStr(pp, pl)) || 'image.png';
          const rgb = new Uint8Array(getMem().buffer, dp, dl);
          const rgba = new Uint8ClampedArray(w * h * 4);
          for (let i = 0, j = 0; j < rgb.length; i += 4, j += 3) {
            rgba[i] = rgb[j];
            rgba[i + 1] = rgb[j + 1];
            rgba[i + 2] = rgb[j + 2];
            rgba[i + 3] = 255;
          }
          const canvas = new OffscreenCanvas(w, h);
          const ctx2 = canvas.getContext('2d');
          ctx2.putImageData(new ImageData(rgba, w, h), 0, 0);
          canvas.convertToBlob({ type: 'image/png' }).then((blob) => {
            const url = URL.createObjectURL(blob);
            const a = document.createElement('a');
            a.href = url;
            a.download = name;
            a.click();
            URL.revokeObjectURL(url);
          });
          return 1;
        } catch (_e) {
          return 0;
        }
      },
    });
  },
);
