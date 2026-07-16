// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// Asyncify controller — the async→sync bridge shared by every --html target
// (GL games, and the headless compute page's store_load_url_trusted fetch).
// A suspend import (loft_gl_swap_buffers / loft_web.ws_yield / loft_io.loft_host_http_get)
// calls ac.suspend() to unwind the whole wasm stack back to the JS event loop;
// the driver later calls ac.resume(fn) to rewind and continue past the yield.
// Usage: const ac = new AsyncifyCtrl(instance);
//        ac.start('loft_start');            // runs until the first suspend
//        if (ac.sleeping) /* pump */ ac.resume('loft_start');
//
// Extracted from loft-gl-wasm.js so the headless template can drive asyncify too
// (one definition, no drift).

function AsyncifyCtrl(instance) {
  // Asyncify data area: an 8-byte struct {current, end} in WASM memory plus a
  // STACK_SIZE save region, allocated right after __heap_base.  `current`
  // (struct field @0) is the live cursor; `end` (@4) caps the save region.
  const DATA_ADDR = (instance.exports.__heap_base?.value || 65536);
  const STACK_SIZE = 16384;  // 16KB for asyncify stack
  const E = instance.exports;
  this.sleeping = false;
  this.exports = E;
  // The save-buffer top after the last unwind.  Asyncify writes the stack
  // UPWARD from the buffer base during an unwind (leaving `current` at the
  // top), and reads it back DOWNWARD during a rewind — so the rewind must
  // start from this saved top, not the base.  Resetting `current` to the base
  // before a rewind loses every saved frame, so the program rewinds into an
  // empty stack and "returns" without ever resuming past the yield (issue
  // #450: the page printed only the first line, then stuck).
  let savedTop = DATA_ADDR + 8;
  const STATE_REWINDING = 2;

  const setStruct = (cur, end) => {
    const mem = new Int32Array(E.memory.buffer);
    mem[DATA_ADDR >> 2] = cur;
    mem[(DATA_ADDR + 4) >> 2] = end;
  };
  const curPtr = () => new Int32Array(E.memory.buffer)[DATA_ADDR >> 2];

  this.start = function(fn) {
    this.sleeping = false;
    E[fn]();
    // suspend() set sleeping + started the unwind; close it out and remember
    // the saved top for the matching rewind.
    if (this.sleeping) {
      savedTop = curPtr();
      E.asyncify_stop_unwind();
    }
  };

  this.resume = function(fn) {
    if (!this.sleeping) return false;
    this.sleeping = false;
    // current = saved top (rewind reads downward); end = buffer top.
    setStruct(savedTop, DATA_ADDR + 8 + STACK_SIZE);
    E.asyncify_start_rewind(DATA_ADDR);
    E[fn]();
    if (this.sleeping) {
      savedTop = curPtr();
      E.asyncify_stop_unwind();
    }
    return true;
  };

  // Called from a suspend import (loft_gl_swap_buffers, loft_web.ws_yield,
  // loft_io.loft_host_http_get) to yield a frame.  The import is re-invoked while
  // asyncify REWINDS — that call is the replay reaching the original yield point,
  // so it must stop_rewind and RETURN, letting execution continue PAST the yield.
  // Re-starting an unwind there (the old bug) spins forever on one iteration:
  // every resume rewound to the yield and immediately re-suspended without
  // making progress.  In the NORMAL state, start an unwind so control returns
  // to the JS event loop.
  this.suspend = function() {
    if (E.asyncify_get_state() === STATE_REWINDING) {
      E.asyncify_stop_rewind();
      return;
    }
    this.sleeping = true;
    setStruct(DATA_ADDR + 8, DATA_ADDR + 8 + STACK_SIZE);
    E.asyncify_start_unwind(DATA_ADDR);
  };
}

export { AsyncifyCtrl };
