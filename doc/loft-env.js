// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// loft#950 — arm a page's `LOFT_*` switches from its query string.
//
// `std::env` on wasm32-unknown-unknown is a stub that always answers nothing, so a
// `--html` page could set none of the 47 switches loft reads — and the store guard's own
// advice, "run with LOFT_STRICT_STORES=1 to name the free", named something the target
// could not do. The module reserves a buffer and exports where it is; this writes the
// settings in and commits them.
//
// ⚠ BEFORE ANY LOFT CODE RUNS. `keys.rs` memoizes each switch on first read, so a setting
// installed after entry is ignored by whichever switch has already been asked — it would
// look like the switch had no effect rather than like it arrived late.
//
// Only `LOFT_`-prefixed parameters are forwarded, so a page's own query string is not
// silently reinterpreted as compiler settings.
function loftInstallEnv(instance, memFallback) {
  const E = instance.exports;
  if (!E.loft_env_buf || !E.loft_env_commit || !E.loft_env_cap) return;
  const q = new URLSearchParams(location.search);
  let blob = "";
  for (const [k, v] of q) if (k.startsWith("LOFT_")) blob += k + "=" + v + "\n";
  if (!blob) return;
  const cap = E.loft_env_cap();
  const bytes = new TextEncoder().encode(blob);
  const n = Math.min(bytes.length, cap);
  const buf = (E.memory || memFallback).buffer;
  new Uint8Array(buf, E.loft_env_buf(), cap).set(bytes.subarray(0, n));
  E.loft_env_commit(n);
}
