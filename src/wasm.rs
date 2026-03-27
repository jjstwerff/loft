// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! WASM entry point and host-bridge stubs for the `wasm` Cargo feature.
//!
//! Compiled only when `--features wasm` is active.  Each host-bridge function
//! corresponds to a JS-side counterpart on `globalThis.loftHost`.
//!
//! Steps: W1.1 (this stub) → W1.2 (output capture) → W1.3–W1.8 (bridges) → W1.9 (entry point).
