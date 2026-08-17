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
  // save region after it.  `current` (struct field @0) is the live cursor;
  // `end` (@4) caps the region.
  //
  // loft#950 — the MODULE owns that memory and tells us where it is.  This used
  // to pick the address here: `__heap_base` when the module exported it, else the
  // literal 65536.  It never did export it (that flag rides in WASM_THREAD_FLAGS,
  // which only the threaded link passes), so every ordinary page took the
  // fallback — and 65536 is not in the heap.  The shadow stack is [0, 0x100000)
  // growing down, so the save region sat 966,648 bytes INTO it: asyncify wrote
  // saved frames over live locals and read live locals back as saved frames.
  //
  // No fallback now.  A module without these exports cannot tell us memory that
  // is safe to write, and guessing is what this bug was.
  const E = instance.exports;
  if (!E.loft_asyncify_data || !E.loft_asyncify_size) {
    throw new Error(
      'loft: this wasm module does not reserve an asyncify save region ' +
      '(no loft_asyncify_data/loft_asyncify_size export) — rebuild the page ' +
      'with a loft that has loft#950');
  }
  const DATA_ADDR = E.loft_asyncify_data();
  const STACK_SIZE = E.loft_asyncify_size();
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

  // loft#950 — a stack deeper than the save region overruns it, and asyncify
  // writes past the end without saying so: the bytes after it belong to another
  // static, so the damage surfaces somewhere else entirely.  The module keeps a
  // canary there and `loft_asyncify_ok` reports it.  Checked after each unwind,
  // which is the only moment the region is written.
  const checkRegion = () => {
    if (E.loft_asyncify_ok && !E.loft_asyncify_ok()) {
      throw new Error(
        'loft: the asyncify save region overflowed — the wasm stack at this ' +
        'suspend needs more than ' + STACK_SIZE + ' bytes.  Everything after ' +
        'this point would be reading corrupted memory.');
    }
  };

  this.start = function(fn) {
    this.sleeping = false;
    E[fn]();
    // suspend() set sleeping + started the unwind; close it out and remember
    // the saved top for the matching rewind.
    if (this.sleeping) {
      savedTop = curPtr();
      E.asyncify_stop_unwind();
      checkRegion();
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
      checkRegion();
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
