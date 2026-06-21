// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN84 ZT-C step 1 — minimal loft_web host.js for the asyncify probe.
// Provides ONLY the ws_yield suspend shim (the dedicated asyncify suspend
// import).  When the wasm calls loft_web.ws_yield(), this triggers the
// AsyncifyCtrl unwind so the driver returns to the event loop and resumes
// on the next setImmediate.  This is exactly the shim the real web/host.js
// installs (minus the socket Map).

(globalThis.LOFT_WASM_EXTENSIONS = globalThis.LOFT_WASM_EXTENSIONS || []).push(
  function loftWebYieldShim(imports, ctrl, _getMem) {
    const ns = (imports.loft_web = imports.loft_web || {});
    ns.ws_yield = function () {
      if (ctrl && ctrl.ac) ctrl.ac.suspend();
    };
  },
);
