<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN84 — Zero-trust shared-files: loft library enablement

Status: **active** (tracking) · Issue: [loft-lang/plans#84](https://github.com/loft-lang/plans/issues/84) · Subject: libs

The loft libraries the **zero-trust-shared-files** consumer
(`/home/jurjen/workspace/zero-trust-shared-files`) needs, taken from its
`DEPENDENCIES.md`. We build these; the project files separate issues for
design / security / protocol asks.

## The master invariant (every lib's acceptance)

*The platform-blind app runs identically native and in the browser — same loft
API, **native == wasm**.* Each lib below is "done" only when its native and wasm
backends are observably interchangeable. (Matches the consumer's runtime model:
loft carries the compute, heavy deps are bridged to the browser's natives.)

## Sequencing

`cbor → crypto wasm-bridge → web wasm-bridge → server TLS → plugin runtime`.
Dependencies: the web-bridge's binary-frame test needs **cbor**; the plugin
browser loader needs the **crypto wasm-bridge** (bridged crypto for the
signature check).

---

## ZT-A — `cbor` (NEW) · home `loft-libs-core/cbor` · → @PLN83

Signable canonical CBOR, built **in pure loft** — no native crate, no
`ciborium`, no FFI bridge (the @PLN83 pivot: a tree-walking FFI codec would
re-introduce cross-binary struct-layout coupling, the wrong risk for a
signature-bearing lib, and CBOR's byte format is simple enough to encode
directly over `vector<u8>`). Full plan + phases C1–C5 in
[`83-cbor.md`](83-cbor.md). Acceptance summary:

- **A1 skeleton + primitives** — `loft.toml` + `src/cbor.loft` + `tests/`; the
  `CborValue` enum; canonical encode for uint / negint / byte-string /
  text-string / bool / null / float64 (shortest-form head). *Check:* RFC 8949
  §Appendix-A primitive vectors byte-identical. (core validated by the @PLN83 probe.)
- **A2 containers + canonical map ordering** — array, map, nesting; **map keys
  sorted by encoded bytes** (RFC 8949 §4.2). *Check:* container + key-ordering
  vectors (out-of-order keys encode sorted). — the load-bearing part.
- **A3 decode** — bytes → `CborValue` with well-formedness checks (loft-safe:
  malformed → error, never a crash). *Check:* decode the RFC corpus;
  `encode→decode→encode` byte-identity; negative tests reject truncated /
  non-canonical / overlong inputs.
  **Status: structural + TEXT decode shipped** (int/negint/bytes/array/bool/null/
  **text** + malformed/non-canonical/trailing rejection), `encode→decode→encode`
  byte-identical on both backends (loft-libs-core@5293cb6). Text decode landed via
  a new stdlib `text_from_bytes(vector<u8>) -> text` primitive (the inverse of
  `byte_at`, loft@8c46955b). **MAP decode now SHIPPED** — the recursive map read
  passes on both backends (cbor lib green, maps included); the struct-with-enum-
  fields append corruption ([loft#406](https://github.com/loft-lang/loft/issues/406))
  and its forward-ref enum↔struct variant ([loft#417](https://github.com/loft-lang/loft/issues/417))
  are both cleared.
- **A4 typed path** — `T.to_cbor()` / `parse_cbor<T>` (loft struct ↔ CBOR map,
  int or text keys, canonical). *Check:* struct corpus round-trips; signed-record
  shape stable.
- **A5 package + publish** — `loft.toml`, registry entry, LIBRARY_CHECKLIST; a
  pinned `native == wasm` guard test (**trivial for pure loft — equality holds
  by construction, no cross-build**). *Check:* `loft install cbor` works;
  checklist green.

**Lib acceptance:** the master `native == wasm` invariant holds **by
construction** (pure loft runs identically on every target), so signature
stability is structural — not proved by a cross-build equality test.

---

## ZT-B — `crypto` wasm-bridge (UPDATE) · home `loft-libs-core/crypto`

Run the **same** crypto API in the browser with a minimal bundle.

> **Status (native primitive suite COMPLETE — the prerequisite B1 didn't list).**
> The lib shipped with only `sha256`/`hmac`/`base64` AND was **broken against the
> current loft binary** (legacy `loft_register!` FFI — plan-74 deleted the
> interpreter marshaller, so even `sha256` panicked "no marshal bridge").
> Migrated the whole lib to the `#[loft_native]` bridge pattern and added the
> full native suite over RustCrypto/dalek: **ed25519** (RFC 8032),
> **x25519_dh** (RFC 7748), **hkdf_sha256** (RFC 5869), **aes256gcm seal/open**
> (Wycheproof), **random_bytes** (OsRng). **51 KAT tests pass on BOTH backends**
> (loft-libs-core@69d1f8b). Text API (base64 bytes), loft-safe (malformed → ""/
> false). **B3 HPKE SHIPPED** — pure-loft RFC 9180 base mode
> (DHKEM(X25519)/HKDF-SHA256/AES-256-GCM) `hpke_seal_base`/`hpke_open_base`,
> byte-exact against the official CFRG vector, 8 KAT tests both backends
> (loft-libs-core@08872d4); added native `hkdf_extract`/`hkdf_expand` + bytes↔
> base64 helpers. Remaining ZT-B is the **wasm bridge** (B1 `crypto.subtle`
> mapping, B2 async↔sync, B4–B6) — now **gated on
> [loft#407](https://github.com/loft-lang/loft/issues/407)**: the `[wasm.bridge]`
> mechanism can't route text-returning natives (pioneered + proved that `sha256`
> bridges + renders in headless chromium *with* a boolean-emit fix + a
> Reference-out reshape; the clean unblocker is a **text-return bridge
> convention** so all ~12 primitives bridge without per-fn reshape). B3 HPKE is
> separately gated on the same bytes↔text gap as cbor text decode.

> **Gating summary (2026-06-21):** **DONE** — crypto native suite + HPKE (59 tests
> both backends), cbor full encode + structural/text/**map** decode (cbor lib green
> on both backends), the `text_from_bytes` enabler, ZT-D1 TLS runbook. **All
> compiler gates CLEARED:** `#405` (loop-local), `#406` (struct-enum vector append),
> `#407` (wasm.bridge text-return), `#408` (registry shadows local routes), `#409`
> (`+=` after FFI alloc) are all **closed**; the forward-ref enum↔struct variant of
> the cbor-map corruption, `#417`, is **fixed** (`fixed-pending-merge`, this branch —
> the one-home `nullable_vector_elem` resolution). **Remaining work has NO compiler
> gate** — it is browser-bridge library development. **B1 reframed + half-shipped
> (2026-06-21):** the `[wasm.bridge]` path compiles a pure-Rust bridge crate to wasm
> — it is NOT a `crypto.subtle`/JS mapping (those are async Promises, which the
> synchronous loft host ABI can't consume → that's B2's asyncify job). The headless
> verify loop is now stood up (node `tools/wasm_repro.mjs`, no browser needed; build
> via `loft --html`). **Shipped + verified `native == wasm`** (same KAT on
> `--interpret` AND on the wasm backend, exit 0): `sha256`, `hmac_sha256`,
> `base64_encode`/`base64_decode`/`base64url_encode`, `hkdf_sha256`/`hkdf_extract`/
> `hkdf_expand` — hashing+base64 SHARED byte-identical via `#[path]` from the native
> zero-dep `sha256.rs`/`base64.rs`, HKDF a pure-Rust RFC-5869 re-impl over the shared
> HMAC (loft-libs-core@9811181). **Curve/AEAD ALSO shipped + verified `native ==
> wasm`** (RFC 8032 / RFC 7748 / Wycheproof KAT on `--interpret` AND wasm, exit 0):
> `ed25519_public_key`/`sign`/`verify`, `x25519_dh`, `aes256gcm_seal`/`open` —
> SHARED byte-identical via `#[path]` from the native `ed25519.rs`/`x25519.rs`/
> `aes256gcm.rs`, with the vetted dalek/RustCrypto crates compiled to
> wasm32-unknown-unknown by the **build-extension** in `src/main.rs` (cargo-build
> the bridge crate's Cargo deps for wasm32, `--extern` the direct deps + `-L` the
> deps dir on the bridge rustc compile and the main wasm link).  The deterministic
> ops need no RNG, so no getrandom / wasm-bindgen.  **B1 COMPLETE — `random_bytes`
> also bridged + verified** via the one HOST-IMPORT path: the synchronous
> `crypto.getRandomValues` exposed by host.js as `loft_crypto.random_fill` (the
> only Web Crypto call that is not a Promise), verified `native == wasm` (correct
> base64 length + CSPRNG-distinct on both backends).  Two loft enablers landed for
> it: the `--html` import-module guard now accepts the `loft_` prefix for
> library-bridge host imports (not just `loft_io`/`loft_gl`), and
> `tools/wasm_repro.mjs` loads extra host.js via `$LOFT_WASM_HOST_JS` so a
> loft-libs-core bridge's host imports test headlessly.  **All 15 crypto primitives
> now bridge native == wasm.**  Next: the web wasm bridge (C2–C3: browser
> `WebSocket` + CBOR frames) and the plugin runtime (ZT-E). ZT-D2 rustls deferred.

- **B1 — `[wasm.bridge]` → `crypto.subtle`.** Map each `n_crypto_*`:
  ed25519 sign/verify, x25519_dh, aes256gcm seal/open, hkdf, sha256, hmac,
  random.
  - *Check:* each bridged primitive passes the **same known-answer vectors** the
    native backend does — RFC 8032 (Ed25519), RFC 7748 (X25519), NIST AES-GCM,
    RFC 5869 (HKDF) — run on the **wasm** backend.
- **B2 — reconcile async↔sync inside the lib** (asyncify suspension and/or a
  batch entry point), so the app's synchronous `verify`/`open` never sees a
  Promise.
  - *Check:* an unmodified loft program calling `verify`/`open` runs on wasm with
    no async leak; a batch-verify of N items completes in **one** host
    round-trip (counted).
- **B3 — HPKE as pure-loft composition** (RFC 9180 base mode over
  `x25519_dh` + `hkdf` + `aes256gcm_seal`); no backend, runs every target.
  - *Check:* HPKE **seal-native → open-wasm** (and reverse) round-trips; matches
    an RFC 9180 test vector.
- **B4 — standardize cross-target AEAD on AES-256-GCM.**
  - *Check:* AES-256-GCM seal/open **native == wasm**; a format test asserts the
    browser-readable format uses only AES-256-GCM (no ChaCha in the cross-target
    path).
- **B5 (optional) — lazy curve25519 fallback wasm** for pre-2025 browsers,
  feature-detected.
  - *Check:* with `subtle` Ed25519/X25519 simulated-absent, the fallback loads +
    matches; with them present, **zero** fallback bytes fetched.
- **B6 — release** native + wasm artifacts.
  - *Check:* `loft install crypto@<new>` pulls both; wasm bundle size recorded.

**Lib acceptance:** sign-native/verify-wasm and seal-native/open-wasm round-trip
— cross-target crypto determinism.

---

## ZT-C — `web` wasm-bridge (UPDATE) · home `loft-libs-net/web`

The WS-client written once must run as the browser client *and* the native
laptop sync agent.

- **C1 — keep the polling WS-client API** (connect, non-blocking `recv`,
  `send`/`send_binary`, `close`).
- **C2 — `[wasm.bridge]` → the browser's native `WebSocket`** (JS holds the
  socket + buffers inbound; sync `recv` drains; `send` hands bytes; frame-yield
  main loop, same async↔sync handling as crypto).
  - *Check:* one client `.loft` source runs unchanged on **both** backends
    (native sync-agent + browser client).
- **C3 — CBOR binary frames (opcode 2) round-trip** on both backends (needs
  ZT-A).
  - *Check:* a CBOR frame sent by a native peer is received **byte-identical** by
    the wasm client and vice versa.

**Lib acceptance:** same WS-client loft code, native == wasm, binary frames
intact.

---

## ZT-D — `server` TLS / ACME · home `loft-libs-net/server`

`server` 0.2.0 is plaintext TCP/WS only; the five HTTPS endpoints need TLS.

- **D1 — reverse-proxy path (now) — DONE (runbook).** `server.listen(port)`
  speaks plaintext HTTP/WS on one port; terminate TLS at a reverse proxy in front
  of it. **Caddy (recommended — automatic ACME, transparent WS upgrade):**
  ```
  files.example.com {
      reverse_proxy 127.0.0.1:8080      # the loft server's listen() port
  }
  ```
  Caddy provisions + renews the cert via ACME (HTTP-01 / TLS-ALPN-01) with no
  extra config, and `reverse_proxy` forwards the WebSocket `Upgrade` frame
  automatically. **nginx (alternative — pair with certbot for ACME):**
  ```
  server {
      listen 443 ssl;
      server_name files.example.com;
      ssl_certificate     /etc/letsencrypt/live/files.example.com/fullchain.pem;
      ssl_certificate_key /etc/letsencrypt/live/files.example.com/privkey.pem;
      location / {
          proxy_pass http://127.0.0.1:8080;
          proxy_http_version 1.1;
          proxy_set_header Upgrade    $http_upgrade;     # WS upgrade
          proxy_set_header Connection "upgrade";
          proxy_set_header Host       $host;
      }
  }
  ```
  - *Check:* `curl https://files.example.com/<endpoint>` returns success over the
    ACME chain; a `wss://` client connects (the `Upgrade`/`Connection` headers
    pass through). All five §9b.3 endpoints inherit TLS with **zero loft code**.
- **D2 — in-loft path (later).** A `rustls` + ACME native bridge so the binary
  provisions its own cert (mirrors the crypto native pattern — not compiler-gated,
  but a large effort; deferred until reverse-proxy friction is shown to matter).
  - *Check:* the loft server serves HTTPS directly with an auto-provisioned cert
    (TLS handshake succeeds end-to-end).

**Lib acceptance:** HTTPS works for the endpoints — D1 now, D2 when reverse-proxy
friction is shown to matter.

---

## ZT-E — plugin runtime (NEW) · home TBD (`loft-libs-net` or a new chunk)

Signed-WASM collaborative-editing plugins, loaded host-side and browser-side
behind the same signature gate.

- **E1 — define the plugin ABI** (`initial_state` / `apply_op` / `make_op` /
  `render` / `snapshot` / `load_snapshot`, per the consumer's PLUGINS.md §9c.2).
  - *Check:* a reference plugin implementing the ABI builds to wasm and exposes
    the six exports.
- **E2 — host loader** (the loft server binary): `wasmtime`-based, verifies the
  module's Ed25519 signature against the trusted-authors list **before**
  instantiation.
  - *Check:* a correctly-signed plugin loads + runs; a tampered / unsigned /
    wrong-key plugin is **rejected** (negative test — the security gate).
- **E3 — browser loader**: `WebAssembly.instantiate` after the **same** Ed25519
  check (using the bridged crypto from ZT-B).
  - *Check:* same accept/reject behaviour on the browser side.

**Lib acceptance:** signed plugins load on host + browser; unsigned / tampered
rejected.

---

## Deferred (tracked, trigger-gated)

- **`crypto` v0.4** — hybrid PQ KEM (X25519 + ML-KEM-768) + Argon2id recovery
  KDF. *Trigger:* before any real deployment leaves long-lived ciphertext at
  rest.
- **MLS (RFC 9420)** over `openmls`. *Trigger:* the first group nears ~20
  members.

## See also

- The consumer's `DEPENDENCIES.md` (source of this list; we do not edit it).
- [`83-cbor.md`](83-cbor.md) — ZT-A in full.
- Project memory `project_zero_trust_shared_files` — the division of labor.
