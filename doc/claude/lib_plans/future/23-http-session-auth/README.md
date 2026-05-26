<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 23 — HTTP response headers + cookie-jar session (native auth enablement)

## Status

**P1 + P2 shipped 2026-05-27.**  P1 = `lib/web` response headers (`headers`
field + `header`/`headers_for`, case-insensitive).  P2 = cookie-jar
`HttpSession` (`http_session(follow_redirects)` + `get`/`post`/`put`/`delete`,
`ureq` `cookies` feature).  10/10 `tests/http.loft` pass on `--interpret`; a
minimal session program (cookie round-trip + header read) verified on
`--native`.  Filed **@P365** along the way (early `return []` miscompiles on
both backends — worked around in `split_headers`).  P3 (base64) open, optional.
Surfaced by the **training port**
(consume-only; their hand-off is
`~/workspace/personal/training/loft/requests/E2-lib-web-http-session.md`) as the
one remaining blocker for **native Garmin login** (their workstream A2,
`loft/sync/GARMIN_NATIVE.md`).  Everything else native-Garmin needs already
works on `lib/web`: data reads + the OAuth2 token *refresh* (A1) shipped.  Only
the interactive *login* is blocked, on exactly two missing capabilities below.

This plan extends the **already-shipped** `lib/web` library (cdylib + `ureq 2.12`,
body returned via a `LAST_BODY` thread-local; see `lib/web/src/web.loft` +
`lib/web/native/src/lib.rs`).  It is the concrete, consumer-driven next slice of
the HTTP client whose broad design lives in
[`../06-web-services/HTTP_CLIENT.md`](../06-web-services/HTTP_CLIENT.md).

## Goal

Add response-header introspection and a cookie-jar HTTP session to `lib/web` so a
loft program can complete a multi-step authenticated login (carry `Set-Cookie`
across requests, read `Location` on redirects) — unblocking native Garmin login
(training A2) end-to-end with no Python.

## Effort + design

- **Effort:** P1 (headers) XS–S; P2 (session) M; overall **M**.
- **Design:** ✓ (the E2 spec is implementation-ready; grounded against the actual
  `ureq`-based code, not the `reqwest` the spec assumed).
- **Last touched:** 2026-05-27

## Sub-arcs

Step-by-step designs (full code + verify commands per step) live in
[IMPL.md](IMPL.md).  All steps are confined to `lib/web` — **no loft-core change**,
because every native signature already has an `src/extensions.rs` auto-marshal arm.

| Item | Steps | Source | Status |
|---|---|---|---|
| **P1** — expose response headers (`header` / `headers_for`) | [P1.1](IMPL.md#step-p11--capture-response-headers-natively-rust-only-no-loft-surface-yet) + [P1.2](IMPL.md#step-p12--expose-headers-to-loft-headers-field--header--headers_for) | E2 spec part 1 | **Shipped 2026-05-27** — the minimum A2 unblock |
| **P2** — cookie-jar `HttpSession` (get/post/put/delete, redirect control) | [P2.1](IMPL.md#step-p21--agent-registry--session-natives-rust-only) + [P2.2](IMPL.md#step-p22--httpsession-loft-api-struct--verbs) | E2 spec part 2 | **Shipped 2026-05-27** — cookie round-trip verified both backends |
| **P3** — `base64_encode`/`decode` (+ `sha256`) | [P3.1](IMPL.md#step-p31--base64_encode--base64_decode-in-libweb) | training nice-to-have #7 | Open — XS, optional, helps OAuth/SSO headers |
| _(related)_ `exec()` subprocess primitive | — | training gap #3 | **Likely moot** — superseded by this workstream |
| _(related)_ TLS/JA3 impersonation (E1) | — | training request #6 | **Deferred** — conditional; "don't build yet" |

## What the consumer (A2 login) actually does

garminconnect's iOS-mobile strategy (a JSON POST, not the HTML/CSRF web flow):

1. `POST <mobile-login>` with credentials → JSON `{ responseStatus.type, … }`.
   **Sets session cookies in the response.**
2. If `type == "MFA_REQUIRED"`: `POST <mfa-complete>` with the user's code **on the
   same session** (cookies from step 1 must be sent back) → JSON with `serviceTicketId`.
3. `POST connectapi.garmin.com/di-oauth2-service/oauth/grant/service_ticket` →
   `di_token` (persist exactly like A1 — already works).

Hard dependency: **cookies set by the credential POST must reach the MFA POST**, and
some steps 302-redirect with a readable `Location`.  Credential/MFA *input* is handled
outside loft (browser HTML form → backend); no stdin needed.

## P1 — response headers (XS–S, additive, zero-risk)

Mirror the existing `LAST_BODY` thread-local exactly:

- Add a `LAST_HEADERS` thread-local + `n_http_headers_raw() -> text` native that
  returns the response headers **newline-joined** in the same `"Name: Value"`
  encoding `join_headers` / `parse_headers` already use on the request side.  (The
  C-ABI auto-marshal can't return a `vector<text>`, so keep the wire form as text and
  split on the loft side — symmetric with how headers are *passed in*.)
- In `http_do`, after `do_request`, populate `LAST_HEADERS` from `resp.headers_names()`
  + `resp.header(name)` (preserve duplicates, e.g. multiple `Set-Cookie`).
- Loft side: add `headers: vector<text>` to `HttpResponse`; `header(name)` (first
  match, case-insensitive, `""` if absent) and `headers_for(name)` (all values).
  Existing `.status`/`.body` callers unaffected.

**This alone unblocks A2** — the caller sets `Cookie:` manually via the existing
`headers` vector between step 1 and step 2.

## P2 — cookie-jar `HttpSession` (M, mechanical — the clean version)

Mirror the `ws_client` slot pattern (a registry keyed by an `i32` handle):

- Flip `ureq = { version = "2", features = ["cookies"] }` — the `cookies` feature
  gates `cookie` + `cookie_store` (confirmed present in 2.12.1).
- Keep a registry of `ureq::Agent`s, each built with a cookie store + a redirect
  policy from the `follow_redirects` argument (`AgentBuilder::redirects(0)` for off).
- Loft side: `http_session(follow_redirects: boolean) -> HttpSession`, struct
  `HttpSession { id }`, methods `get`/`post`/`put`/`delete(self, url, [body,] headers)`.
  Reuse `n_http_body` / `n_http_headers_raw` for the response.  Stateless `http_*`
  functions stay as-is.

Step-1→step-2 MFA cookies then carry automatically; `follow_redirects=false` lets the
caller read `Location` (the ticket step 302s) while keeping the jar.

## P3 — base64 / sha256 (optional)

`base64` is already vendored in `lib/web` / `lib/server`; exposing `base64_encode` /
`base64_decode` (and a `sha256`) helps A2's OAuth/SSO header construction.  XS.  The
port has a pure-loft base64 workaround today, so this is not blocking — fold in only
if convenient.

## Phase ordering

Full per-step designs + verify commands: [IMPL.md](IMPL.md).

1. **P1** (steps P1.1 → P1.2) first — XS–S, zero-risk, the minimum unblock.  Ship
   it, let the training port verify native login with manual cookie handling.
2. **P2** (steps P2.1 → P2.2) next — the clean version; promotes manual cookie
   handling to an automatic jar.  Depends on P1 (P2.2 reuses `http_headers_raw`).
3. **P3** (step P3.1) opportunistically alongside either.

Each step is independently verifiable (its own build + loft test); P1 and P2 each
land as one commit per [IMPL.md § Landing order](IMPL.md#landing-order--commits).

## Open design questions — RESOLVED (consumer review 2026-05-26)

The training port reviewed this plan and confirmed it meets request E2 in full;
all three questions are answered (E2 spec § "Consumer review of @PLAN23-lib"):

1. **Case-insensitive header lookup** — ✅ required; the consumer reads
   `set-cookie` / `location` by name.  Lowercased names in the stored blob are
   fine (values keep their casing).  *(Resolved: keep the `ascii_lower` +
   lowercased-name design.)*
2. **Session lifetime / cleanup** — ✅ **process-lifetime jar, no `close()`.**
   Login is short-lived (a few requests, one process).  Add `close()` later only
   if a leak shows up — do **not** block P2 on it.
3. **WASM target** — ✅ **out of scope is correct.**  Login runs native (the
   browser can't call Garmin directly — CORS — so the backend does the HTTP); P1
   headers on WASM are not required.

**Phasing confirmed by the consumer: ship P1 first** — they wire native login
with manual `Cookie:` handling and verify end-to-end, then adopt P2 when it
lands.  P3 stays optional (they have a pure-loft base64).

## Cross-arc dependencies

- [`../06-web-services/`](../06-web-services/) — this plan is the concrete first
  maturation of that umbrella's HTTP_CLIENT sub-plan; design content stays linked, not
  duplicated.
- [`../08-server/`](../08-server/) — server-side already vendors `base64`; P3's
  helper would be shared stdlib, useful to both.
- Training request **E1** (TLS/JA3 impersonation) is a *separate, conditional*
  follow-on — only if Garmin's WAF blocks the plain-TLS credential POST (untested).
  Not part of this plan.

## See also

- [IMPL.md](IMPL.md) — the small verifiable implementation steps with full code.
- [`../06-web-services/HTTP_CLIENT.md`](../06-web-services/HTTP_CLIENT.md) — the
  broad HTTP-client design this plan implements a slice of.
- `~/workspace/personal/training/loft/requests/E2-lib-web-http-session.md` — the
  consumer's implementation-ready spec + acceptance criteria (consume-only; do not
  edit the training repo).
- [`../../../ROADMAP.md`](../../../ROADMAP.md) — schedules this plan's items.
