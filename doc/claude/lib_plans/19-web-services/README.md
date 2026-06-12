<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Web Services — client-side library (@PLN19)

**Tracker: [loft-lang/plans#19](https://github.com/loft-lang/plans/issues/19)** —
lifecycle state lives on the issue, not in this path.

Long-term plan for a **fully functioning client-side web
services library** in loft.  Server-side HTTP / WebSocket /
TLS work lives separately in `WEB_SERVER_LIB.md` (still at
the doc root, awaiting its own promotion).

## Status

Mixed status — split deliberately so each file has a clear
single concern:

| Sub-plan | File | Status |
|---|---|---|
| JSON serialization + deserialization | [JSON.md](JSON.md) | **Shipped** — `Type.parse()` + `:j` format flag work today; re-verified hands-on 2026-06-12 against a real ~100-field GitHub API payload (unknown-field tolerance, nested structs, JSON `null` → loft null caught with `??`, `:j` round-trip, dynamic `json_parse` navigation with safe `JNull` on missing fields) |
| HTTP client (verbs + headers) | [HTTP_CLIENT.md](HTTP_CLIENT.md) | **Planned** — locked-in design; deferred to 1.1+ per ROADMAP H4 |
| Future expansions | (this file, see below) | Sketched only — no scheduled work |

The JSON layer is reference documentation for capabilities
already in production; the HTTP layer is the primary planned
work; the future-expansions section sketches what else a
"fully functioning" library would cover.

The 2026-06-12 "loft as a web-service reader" evaluation
sharpened the priority: loft is **parse-strong, fetch-missing**.
There is no way to perform an HTTP request from loft (and no
subprocess primitive to shell out to `curl` —
[`15-process`](../future/15-process/README.md)), so the only
workflow today is a bash wrapper fetching into a file that loft
parses.  Ship-order step 1 (`http_get` + friends) is the single
highest-leverage unlock; per the dogfood loop a small real
consumer (e.g. a GitHub-API CLI tool) should drive it.

## Scope

This plan is **client-side**.  A loft program runs HTTP
requests against external services, parses responses (JSON
or otherwise), and acts on them.  Common shapes:

- CLI tools that call REST APIs
- Server-side glue that orchestrates calls to other services
- Game clients fetching JSON config / leaderboards / asset
  manifests
- Build tooling that posts release notes, fetches dependency
  metadata, etc.

**Out of scope** (covered by other plans):

- HTTP server, WebSockets, TLS termination, authentication,
  RBAC, ACME — see `WEB_SERVER_LIB.md` (next library plan
  promotion).
- Game-client multiplayer protocol — see
  `GAME_CLIENT_LIB.md` and the EVENT_LOOP plan
  ([../../plans/future/23-event-loop/](../../plans/future/23-event-loop/)).
- WASM HTTP fetch in the browser — currently routed through
  the host's `fetch()` via the W1.x bridge; would inherit
  this design's `HttpResponse` shape but uses a different
  native implementation.

## Sub-files

- **[JSON.md](JSON.md)** — currently-shipped JSON
  serialization / deserialization reference.  Includes a
  "future JSON extensions" sketch (schema validation, JSON
  Pointer, streaming parser, JSON Patch, etc.) — none
  scheduled, all sketched for future consideration.
- **[HTTP_CLIENT.md](HTTP_CLIENT.md)** — locked-in HTTP
  client design.  `HttpResponse` struct + plain `http_*`
  functions, `ureq`-backed, feature-gated.  5-step
  implementation plan.  Comparison with original 2026-03-18
  approaches (which are now obsolete thanks to `Type.parse`).

## Future expansions — toward a "fully functioning" web-services library

HTTP-verb calls + JSON round-trip cover the common case but
fall short of a full client library.  This section catalogs
what else a "fully functioning" library would offer.  None
are scheduled.  Listed here so each future session knows the
shape of the eventual scope, and so consumers can request
specific items when their use case appears.

### URL handling

| Feature | Why it matters |
|---|---|
| URL parsing (`scheme`, `host`, `port`, `path`, `query`, `fragment`) | Building / inspecting URLs without string manipulation |
| Query-string encoding / decoding | Safe param assembly: `url_with_params(base, [(k, v), …])` |
| Path encoding | Percent-encoding for non-ASCII path segments |
| URL normalisation | Trailing-slash, default-port, case-folding for comparison |

### Request / response refinements

| Feature | Why it matters |
|---|---|
| Streaming response body (`for chunk in resp.stream()`) | Large downloads without loading entire body into memory |
| Streaming request body | Uploading large files without buffering |
| Multipart/form-data uploads | File-upload to APIs that don't accept JSON |
| Content negotiation helpers | `accept_json()` / `accept_html()` / etc. set the right `Accept` headers |
| MIME type registry | Mapping extensions ↔ content types |
| Conditional requests (ETag / Last-Modified / If-None-Match) | Cache validation without re-downloading unchanged resources |
| Retry + exponential backoff | Resilience against transient failures |
| Per-request timeouts | Prevent hangs on slow upstreams |
| Connection pooling | Reuse TCP/TLS for repeat calls to the same host |

### Authentication helpers

| Feature | Why it matters |
|---|---|
| Bearer-token convenience (`http_get_bearer(url, token)`) | Shorter than building the header vector by hand |
| HTTP Basic auth helper | `http_get_basic(url, user, pass)` |
| OAuth2 client credentials flow | Programmatic API access without user interaction |
| OAuth2 authorization-code flow | User-facing tools that need user consent |
| Cookie store | Server sessions across multiple requests |
| Signed-request helpers (HMAC, request signing) | AWS SigV4, GitHub webhook verification, etc. |

### Real-time / push

| Feature | Why it matters |
|---|---|
| Server-Sent Events (SSE) client | Subscribe to event streams (LLM token streams, log tails) |
| WebSocket client | Bidirectional real-time (chat, game state sync, collaborative editing).  Server-side WebSocket lives in `WEB_SERVER_LIB.md`; client-side belongs here. |
| HTTP/2 push handling | When servers send pre-emptive responses |

### Transport / TLS

| Feature | Why it matters |
|---|---|
| Custom CA bundles | Corporate certificate authorities |
| Client certificates (mTLS) | API gateways that require client identity |
| Cipher / version pinning | Compliance-driven TLS configuration |
| HTTP proxy support | Corporate networks |
| `socks5://` proxy support | Tor / privacy-tooling |
| DNS resolution overrides | `/etc/hosts`-style or per-process DNS |

### Diagnostic / dev experience

| Feature | Why it matters |
|---|---|
| Request / response logging hook | Debugging without a separate proxy tool |
| Mock client for tests | Unit-test code that calls `http_*` without hitting the network |
| Recording / playback (VCR-style) | Reproducible integration tests |
| Curl-equivalent CLI dump | "Show me the curl command this would run" for debugging |

### JSON-adjacent formats

The same `Type.parse()` mechanism could underlie:

| Format | Implementation hook |
|---|---|
| Form-urlencoded (request bodies) | `Type.parse(body, format: form)` overload |
| YAML | `Type.parse(body, format: yaml)` — yaml-rust crate |
| TOML | Mostly for config; could share parser infrastructure |
| MessagePack / CBOR | Binary serialization for performance-sensitive paths |

## Ship order

When the H tier opens (1.1+ or earlier per release decisions),
the suggested ordering is:

1. **HTTP client basics** — Steps 1–5 of
   [HTTP_CLIENT.md § Implementation plan](HTTP_CLIENT.md#implementation-plan).
   Lands `http_get` / `http_post` / `http_put` / `http_delete`
   + the `_h` header variants.
2. **URL handling** — `URL` struct + parser, `url_with_params`
   helper.  Required by anything more sophisticated than
   string-concat URLs.
3. **Auth helpers** — Bearer / Basic shorthand, then OAuth2
   client-credentials flow.  Cookie store comes when first
   consumer needs sessions.
4. **Streaming** — request + response body streaming.
   Triggered by first consumer hitting a payload that
   doesn't fit in memory.
5. **WebSocket / SSE client** — real-time push.  Triggered
   by the multiplayer-editor / game-client work or by an
   LLM-streaming consumer.
6. **TLS refinements** — custom CAs, mTLS, proxies.
   Triggered by deployment context (corporate networks,
   compliance shops).
7. **Diagnostic tooling** — mock client, recording, curl
   dump.  Comes when the test surface for HTTP-using code
   gets painful.
8. **Adjacent formats** — form-urlencoded, YAML, etc.
   Triggered per consumer.

Each step is independent — order can shift based on what the
first real consumer actually needs.

## See also

- [JSON.md](JSON.md) — currently-shipped JSON capabilities
- [HTTP_CLIENT.md](HTTP_CLIENT.md) — HTTP client design
- [../../STDLIB.md](../../STDLIB.md) — stdlib reference
- [../../LOFT.md](../../LOFT.md) — language reference
- [../../PLANNING.md](../../PLANNING.md) — H-tier items in the backlog
- [../../ROADMAP.md](../../ROADMAP.md) — milestone placement
- `../../WEB_SERVER_LIB.md` (still at doc root) —
  server-side counterpart; covers HTTP server, WebSockets,
  TLS, ACME, auth, RBAC.
