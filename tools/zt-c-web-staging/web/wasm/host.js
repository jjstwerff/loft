// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// Browser-WASM host imports for the `web` library's `--html` build (@PLN84
// ZT-C).  Concatenated into the generated HTML preamble by the `--html`
// driver (which reads `[wasm.bridge].host_js = "wasm/host.js"` from
// loft.toml), and loaded headlessly by tools/wasm_ws_repro.mjs.
//
// Provides the `loft_web` host-import namespace the bridge crate declares:
// a live `WebSocket` per handle plus an inbound frame queue.  The wasm side
// polls (ws_poll) and reads the latched current frame (ws_msg_len/copy/
// opcode).  ws_yield is the asyncify suspend shim — between an unwind and the
// next resume the JS event loop runs, which is when WebSocket.onmessage fires
// and fills `inbound`.
//
// Re-fetch `getMem().buffer` in EVERY import — wasm memory can grow/detach.
// `.slice()` payloads before WebSocket.send so we never hand the socket a view
// into wasm memory that could move.

(globalThis.LOFT_WASM_EXTENSIONS = globalThis.LOFT_WASM_EXTENSIONS || []).push(
  function loftWebHostImports(imports, ctrl, getMem) {
    const ns = (imports.loft_web = imports.loft_web || {});
    const enc = new TextEncoder();
    const dec = new TextDecoder('utf-8', { fatal: false });

    // handle -> { socket, inbound: [{op, bytes}], current: {op, bytes} | null }
    const conns = new Map();
    let nextHandle = 0;

    function bytesAt(ptr, len) {
      return new Uint8Array(getMem().buffer, ptr, len);
    }

    ns.ws_connect = function (urlPtr, urlLen) {
      let url;
      try {
        url = dec.decode(bytesAt(urlPtr, urlLen));
      } catch (_e) {
        return -1;
      }
      let sock;
      try {
        sock = new WebSocket(url);
      } catch (_e) {
        return -1;
      }
      sock.binaryType = 'arraybuffer';
      const h = nextHandle++;
      const conn = { socket: sock, inbound: [], current: null, open: false, closed: false };
      conns.set(h, conn);
      sock.onopen = () => {
        conn.open = true;
      };
      sock.onmessage = (ev) => {
        if (typeof ev.data === 'string') {
          conn.inbound.push({ op: 1, bytes: enc.encode(ev.data) });
        } else {
          conn.inbound.push({ op: 2, bytes: new Uint8Array(ev.data) });
        }
      };
      sock.onclose = () => {
        conn.closed = true;
      };
      sock.onerror = () => {
        conn.closed = true;
      };
      return h;
    };

    ns.ws_send = function (h, ptr, len) {
      const conn = conns.get(h);
      if (!conn || conn.socket.readyState !== 1 /* OPEN */) return 0;
      // Send a STRING -> TEXT frame.  Decode the wasm bytes as UTF-8.
      const text = dec.decode(bytesAt(ptr, len).slice());
      try {
        conn.socket.send(text);
        return 1;
      } catch (_e) {
        return 0;
      }
    };

    ns.ws_send_binary = function (h, ptr, len) {
      const conn = conns.get(h);
      if (!conn || conn.socket.readyState !== 1) return 0;
      // Send a Uint8Array -> BINARY frame.  `.slice()` detaches from wasm
      // memory.  Sending binary as a binary frame is load-bearing for C3 — a
      // CBOR payload sent as TEXT would mangle its zero bytes.
      const buf = bytesAt(ptr, len).slice();
      try {
        conn.socket.send(buf);
        return 1;
      } catch (_e) {
        return 0;
      }
    };

    ns.ws_poll = function (h) {
      const conn = conns.get(h);
      if (!conn) return 0;
      const next = conn.inbound.shift();
      if (!next) {
        conn.current = null;
        return 0;
      }
      conn.current = next;
      return 1;
    };

    ns.ws_msg_len = function (h) {
      const conn = conns.get(h);
      return conn && conn.current ? conn.current.bytes.length : 0;
    };

    ns.ws_msg_copy = function (h, ptr) {
      const conn = conns.get(h);
      if (!conn || !conn.current) return;
      const src = conn.current.bytes;
      // Re-fetch the buffer (it may have grown since ws_msg_len).
      new Uint8Array(getMem().buffer, ptr, src.length).set(src);
    };

    ns.ws_opcode = function (h) {
      const conn = conns.get(h);
      return conn && conn.current ? conn.current.op : 1;
    };

    ns.ws_close = function (h) {
      const conn = conns.get(h);
      if (!conn) return;
      try {
        conn.socket.close();
      } catch (_e) {
        /* already closing */
      }
      conns.delete(h);
    };

    // Asyncify suspend: hand control back to the JS event loop for one frame.
    // `ctrl.ac` is the AsyncifyCtrl (the --html preamble / wasm_ws_repro.mjs
    // sets it after instantiate).  No-op if asyncify is absent (compute-only
    // bundle).
    ns.ws_yield = function () {
      if (ctrl && ctrl.ac) ctrl.ac.suspend();
    };
  },
);
