// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# Web Server Library

Design for `server` — a fully featured HTTP server library for loft programs.
The library is mostly written in loft itself; a thin native Rust layer handles
the parts that cannot be expressed without OS-level I/O or cryptographic
primitives.

---

## Contents
- [Goals](#goals)
- [What is in loft vs native](#what-is-in-loft-vs-native)
- [Package structure](#package-structure)
- [Core types](#core-types)
- [Server configuration](#server-configuration)
- [Application setup and routing](#application-setup-and-routing)
- [Middleware](#middleware)
- [Request body parsing](#request-body-parsing)
- [WebSockets](#websockets)
- [TLS — static certificates](#tls--static-certificates)
- [TLS — ACME / Let's Encrypt](#tls--acme--lets-encrypt)
- [Authentication](#authentication)
- [Authorization](#authorization)
- [Multi-threading model](#multi-threading-model)
- [Static file serving](#static-file-serving)
- [Complete example](#complete-example)
- [Native layer boundary](#native-layer-boundary)
- [Implementation phases](#implementation-phases)
- [Dependencies](#dependencies)
- [Game server additions](#game-server-additions)

---

## Goals

1. A loft program can serve HTTP requests with a few lines of setup code.
2. The majority of routing, middleware, auth, and authorization logic is
   written in ordinary loft — readable, testable, and modifiable without
   touching Rust.
3. HTTPS is supported out of the box: static PEM certificates for
   production deployments, automatic Let's Encrypt certificates for the
   common case.
4. WebSockets are first-class.
5. Authentication (JWT, session, API key, HTTP Basic) and role-based
   authorization are built in, not bolted on.
6. The library is distributed as a separate GitHub repository and installed
   via `loft install server` — it is not part of the interpreter.

---

## What is in loft vs native

### Implemented in loft

- Route pattern matching (parse `{param}` and `{wild:*}` segments, match
  against request paths, extract named parameters)
- Middleware pipeline execution (apply a list of `Middleware` values in
  order; short-circuit on non-null response)
- Request body helpers (`parse_json`, `parse_form`, `parse_multipart`)
- JWT payload decoding (base64url decode header + claims, JSON parse —
  cryptographic *verification* of the signature is native)
- Session cookie issuance and validation (generate/parse session ID cookies,
  look up session data in the session store)
- Authorization logic (check `req.roles` against required roles)
- CORS header computation
- Rate-limit counter tracking (using loft's parallel store for thread safety)
- Static file serving (read file, choose Content-Type from extension, set
  headers, stream body as text)
- Response builder functions (`response_ok`, `response_json`, etc.)
- All struct and enum type definitions

### Native Rust (unavoidable)

- TCP socket listen + accept loop (`tokio::net::TcpListener`)
- HTTP/1.1 framing and keep-alive (`hyper`)
- TLS termination (`rustls`)
- WebSocket frame encode/decode (`tokio-tungstenite`)
- HMAC-SHA256 JWT signature verification (`hmac` + `sha2`)
- ACME protocol state machine + HTTP-01 / TLS-ALPN-01 challenges
  (`instant-acme`)
- Argon2 password hash verification (`argon2`)
- Thread pool bridging between async tokio and blocking loft handlers

---

## Package structure

```
server/                        ← GitHub: jjstwerff/loft-server
  loft.toml
  src/
    server.loft                ← Server config struct, serve()
    router.loft                ← Route matching, App struct, route()
    request.loft               ← Request, Header, Cookie, Param types
    response.loft              ← Response struct and builder functions
    middleware.loft            ← Middleware enum, built-in middleware logic
    body.loft                  ← parse_json(), parse_form()
    websocket.loft             ← WebSocket type, ws_send/receive
    auth.loft                  ← AuthConfig enum, auth middleware logic
    authz.loft                 ← RouteGuard, require_roles()
    session.loft               ← Session struct, session store logic
    tls.loft                   ← TlsConfig, AcmeConfig
    static_files.loft          ← serve_dir(), content type helpers
  native/
    libloft_server.so          ← Linux (TCP, TLS, WS, ACME, crypto)
    libloft_server.dylib       ← macOS
    loft_server.dll            ← Windows
  tests/
    routing.loft
    middleware.loft
    auth.loft
    websocket.loft
```

---

## Core types

```loft
struct Header {
    name:  text,
    value: text,
}

struct Param {
    name:  text,
    value: text,
}

struct Cookie {
    name:     text,
    value:    text,
    path:     text = "/",
    domain:   text,
    max_age:  integer,          // seconds; 0 = session cookie
    secure:   boolean = true,
    http_only: boolean = true,
    same_site: text = "Strict", // "Strict", "Lax", "None"
}

struct Request {
    method:    text,            // "GET", "POST", …
    path:      text,            // "/users/42"
    query:     text,            // raw query string "page=1&limit=10"
    headers:   vector<Header>,
    cookies:   vector<Cookie>,
    body:      text,
    remote:    text,            // client IP address
    // Populated by routing:
    params:    vector<Param>,   // path parameters from pattern e.g. {id}
    // Populated by authentication middleware:
    principal: text,            // authenticated identity (null if anonymous)
    roles:     vector<text>,    // authorization roles granted to principal
    session:   text,            // session ID (null if no session)
    // General-purpose middleware attachment:
    attrs:     vector<Param>,   // arbitrary key/value pairs
}

struct Response {
    status:  integer,
    headers: vector<Header>,
    cookies: vector<Cookie>,
    body:    text,
}
```

### Request accessors (implemented in loft)

```loft
// Look up a path parameter set by the router (e.g. {id} in /users/{id}).
pub fn param(req: Request, name: text) -> text

// Look up a query-string parameter (parses req.query on each call).
pub fn query(req: Request, name: text) -> text

// Look up a header value (case-insensitive name lookup).
pub fn header(req: Request, name: text) -> text

// Look up a cookie.
pub fn cookie(req: Request, name: text) -> text

// Look up a middleware attribute (set by auth or custom middleware).
pub fn attr(req: Request, name: text) -> text

// True if the request carries a valid authenticated principal.
pub fn is_authenticated(req: Request) -> boolean

// True if the principal has the given role.
pub fn has_role(req: Request, role: text) -> boolean
```

### Response builder functions (implemented in loft)

```loft
pub fn response_ok(body: text) -> Response
pub fn response_json(json: text) -> Response   // sets Content-Type application/json
pub fn response_html(html: text) -> Response   // sets Content-Type text/html
pub fn response_created(body: text) -> Response
pub fn response_no_content() -> Response
pub fn response_bad_request(msg: text) -> Response
pub fn response_unauthorized() -> Response
pub fn response_forbidden() -> Response
pub fn response_not_found() -> Response
pub fn response_error(msg: text) -> Response   // 500 Internal Server Error
pub fn response_redirect(url: text) -> Response        // 302
pub fn response_redirect_permanent(url: text) -> Response  // 301
pub fn response_status(status: integer, body: text) -> Response

// Fluent modifiers return a new Response.
pub fn with_header(res: Response, name: text, value: text) -> Response
pub fn with_cookie(res: Response, cookie: Cookie) -> Response
pub fn without_cookie(res: Response, name: text) -> Response  // expire cookie
```

---

## Server configuration

```loft
struct Server {
    host:            text = "0.0.0.0",
    port:            integer = 8080,
    tls:             TlsConfig,         // defaults to TlsConfig.None
    threads:         integer = 0,       // 0 = CPU core count
    max_connections: integer = 1000,
    read_timeout:    integer = 30,      // seconds
    write_timeout:   integer = 30,      // seconds
    max_body:        integer = 1048576, // 1 MB; 0 = unlimited
    access_log:      boolean = true,
}

enum TlsConfig {
    None,
    Pem  { cert_file: text, key_file: text },
    Acme { config: AcmeConfig },
}

struct AcmeConfig {
    domains:   vector<text>,    // primary domain first
    email:     text,            // contact address for Let's Encrypt
    storage:   text = "/var/lib/loft/certs/",
    staging:   boolean = false, // true = use Let's Encrypt staging
    challenge: text = "http-01",   // "http-01" or "tls-alpn-01"
    renew_days: integer = 30,   // renew this many days before expiry
}
```

---

## Application setup and routing

```loft
// App is the central object.  Created once in main(), configured,
// then passed to serve().
struct App { /* opaque; backed by router.loft internals */ }

// Create an App with the given server configuration.
pub fn new_app(server: Server) -> App

// Register a route.  method is "GET", "POST", "PUT", "DELETE", etc.,
// or "*" to match any method.
//
// Pattern segments:
//   /users/{id}       — named path parameter
//   /files/{path:*}   — wildcard: matches rest of path including slashes
//   /exact/path       — literal match
//
// Handler receives the full Request (including populated params field)
// and returns a Response.
pub fn route(
    app:     &App,
    method:  text,
    pattern: text,
    handler: fn(Request) -> Response,
)

// Shorthand route registrations.
pub fn get(app: &App, pattern: text, handler: fn(Request) -> Response)
pub fn post(app: &App, pattern: text, handler: fn(Request) -> Response)
pub fn put(app: &App, pattern: text, handler: fn(Request) -> Response)
pub fn delete(app: &App, pattern: text, handler: fn(Request) -> Response)

// Register a WebSocket upgrade handler.  The handler is called after
// the handshake; it owns the WebSocket for its entire lifetime.
pub fn route_ws(
    app:     &App,
    pattern: text,
    handler: fn(Request, &WebSocket),
)

// Register a static file directory.  GET /prefix/file.txt serves
// <dir>/file.txt.  Directory listing is disabled by default.
pub fn serve_dir(app: &App, prefix: text, dir: text)

// Attach a middleware.  Middleware are applied in registration order.
pub fn use_middleware(app: &App, mw: Middleware)

// Start the server.  Blocks until the process is killed or
// an unrecoverable error occurs.
pub fn serve(app: App)
```

### Route matching (implemented in loft)

Routes are matched in registration order.  The first match wins.  Pattern
matching is implemented entirely in loft:

```
/users/42        matches  /users/{id}    → params = [Param { name="id", value="42" }]
/files/a/b/c     matches  /files/{p:*}   → params = [Param { name="p", value="a/b/c" }]
/health          matches  /health        → params = []
```

If no route matches, the server returns a 404 response generated by a
default handler (overridable via `route(app, "*", "/*", fn my_404)`).

---

## Middleware

Middleware run before the route handler.  Each middleware in the list is
tested in order; if it returns a non-null response, the remaining
middleware and the handler are skipped.  This allows middleware to
short-circuit (e.g. return 401 before calling the handler).

```loft
enum Middleware {
    // Log each request: method, path, status, duration.
    Logger,

    // Add CORS headers.  origins = ["*"] allows all origins.
    Cors {
        origins: vector<text>,
        methods: vector<text>,
        headers: vector<text>,
        max_age: integer,
    },

    // Reject requests that exceed max_rps from a single IP.
    // Counting is per-IP, stored in loft's parallel-safe store.
    RateLimit {
        max_rps: integer,
        window:  integer,   // sliding window in seconds
        by:      text,      // "ip" (default) or header name (e.g. "X-API-Key")
    },

    // Decompress request body (gzip, deflate, br).
    Decompress,

    // Set security response headers (CSP, HSTS, X-Frame-Options, etc.).
    SecureHeaders,

    // Authenticate the request and populate req.principal and req.roles.
    // Does NOT reject anonymous requests — that is done by RequireAuth.
    Authenticate { config: AuthConfig },

    // Reject unauthenticated requests (req.principal == null) with 401.
    RequireAuth,

    // Reject requests where the principal does not have all required roles.
    RequireRoles { roles: vector<text> },

    // Add req.attrs entry from a request header value.
    ForwardHeader { header: text, attr: text },
}
```

### Why an enum rather than function chains

Closures that capture variables are not yet supported in loft (deferred to
1.1+ as A5).  An enum-based middleware registry avoids this limitation while
remaining fully declarative.  Each middleware variant carries its
configuration inline.

When A5 lands, a function-based alternative will be possible:

```loft
// Future (post-A5):
pub fn mw_rate_limit(config: RateLimitConfig) -> fn(Request) -> Response?
```

For now, `Middleware` variants cover all common cases.  Application-specific
logic belongs in route handlers, not middleware.

---

## Request body parsing

Implemented in loft using the existing `Type.parse()` mechanism for JSON
and simple text parsing for form encoding.

```loft
// Parse request body as a JSON struct.
// Returns null if the body is not valid JSON for type T.
pub fn parse_json(req: Request) as T -> T

// Parse application/x-www-form-urlencoded body.
// Returns vector of Param { name, value }.
pub fn parse_form(req: Request) -> vector<Param>

// Convenience: get a single form field.
pub fn form_field(req: Request, name: text) -> text
```

Usage:

```loft
struct CreateUser { name: text, email: text }

fn handle_create(req: Request) -> Response {
    user = parse_json(req) as CreateUser;
    if user == null {
        return response_bad_request("invalid JSON");
    }
    // ... store user ...
    response_created("{user:j}")
}
```

---

## WebSockets

WebSocket frame encoding/decoding is native.  The loft API is a simple
blocking send/receive interface.

```loft
struct WebSocket { /* opaque */ }

enum WsMessage {
    Text   { content: text },
    Binary { data: text },    // raw bytes represented as base64
    Ping   { data: text },
    Pong   { data: text },
    Close  { code: integer, reason: text },
}

// Send a message.  Blocks until the frame is written to the OS buffer.
pub fn ws_send(ws: &WebSocket, msg: WsMessage)

// Receive the next message.  Blocks until a frame arrives or the
// connection closes.  Returns WsMessage.Close when the peer closes.
pub fn ws_receive(ws: &WebSocket) -> WsMessage

// Close the connection gracefully.
pub fn ws_close(ws: &WebSocket, code: integer, reason: text)

// True if the connection is still open.
pub fn ws_is_open(ws: &WebSocket) -> boolean
```

### WebSocket handler pattern

```loft
fn handle_chat(req: Request, ws: &WebSocket) {
    user = req.principal ?? "anonymous";
    for msg in ws {     // syntactic sugar for ws_receive loop (future)
        match msg {
            Text { content } => ws_send(ws, WsMessage.Text { content: "{user}: {content}" }),
            Close { code, reason } => break,
            _ => {},
        }
    }
}
```

Until the `for msg in ws` iterator syntax is supported, write the loop
explicitly:

```loft
fn handle_chat(req: Request, ws: &WebSocket) {
    user = req.principal ?? "anonymous";
    running = true;
    for _ in 0..MAX_INT if running {
        msg = ws_receive(ws);
        match msg {
            Text { content } =>
                ws_send(ws, WsMessage.Text { content: "{user}: {content}" }),
            Close { _, _ } => running = false,
            _ => {},
        }
    }
}
```

---

## TLS — static certificates

```loft
server = Server {
    port: 443,
    tls: TlsConfig.Pem {
        cert_file: "/etc/ssl/certs/example.com.pem",
        key_file:  "/etc/ssl/private/example.com.key",
    },
};
```

The native layer loads the certificate and private key at startup using
`rustls`.  Supported formats: PEM (X.509 certificate chain + PKCS#8 or
RSA private key).  PKCS#12 is not supported directly; convert with
`openssl pkcs12 -in bundle.p12 -out cert.pem -nodes`.

Certificate reloading without restart is not supported in Phase 1 (deferred
to a future `reload_certs()` call that the application can trigger on SIGHUP).

---

## TLS — ACME / Let's Encrypt

ACME (Automatic Certificate Management Environment, RFC 8555) allows the
server to obtain and automatically renew TLS certificates from Let's Encrypt
or any compatible CA.

```loft
server = Server {
    port: 443,
    tls: TlsConfig.Acme {
        config: AcmeConfig {
            domains:    ["example.com", "www.example.com"],
            email:      "admin@example.com",
            storage:    "/var/lib/loft/certs/",
            staging:    false,
            challenge:  "http-01",
            renew_days: 30,
        },
    },
};
```

### ACME flow (handled by native layer)

1. **Startup**: check `storage/` for a cached certificate.  If found and
   valid for more than `renew_days` days, use it immediately.
2. **Initial issuance** (no cached cert or cert too close to expiry):
   - Register an account with the ACME directory (once; account key cached).
   - Submit an order for each domain.
   - **HTTP-01 challenge**: start listening on port 80 and serve the
     `.well-known/acme-challenge/<token>` path; or **TLS-ALPN-01**: respond
     to TLS connections with a challenge certificate on the `acme-tls/1`
     ALPN protocol.
   - Wait for the CA to validate each challenge.
   - Download the issued certificate chain and private key.
   - Save to `storage/` and activate.
3. **Automatic renewal**: a background goroutine checks daily; if the
   certificate is within `renew_days` of expiry, it repeats the issuance
   flow and swaps the certificate live without restarting.

### HTTP-01 challenge and port 80

For HTTP-01 challenges, the native layer also starts a plain HTTP listener
on port 80 temporarily.  If the application already has a port-80 listener
(for redirect), the challenge path is served before the redirect handler.
This requires the server process to have permission to bind port 80 (run as
root or with `CAP_NET_BIND_SERVICE`, or use a port redirect at the OS level).

### Requirements

- Public DNS for each domain must resolve to the server's IP before startup.
- Ports 80 and 443 must be reachable from the internet for challenge validation.
- `storage/` must be writable and persistent across restarts.

---

## Authentication

Authentication middleware populates `req.principal` and `req.roles`.  It
does not reject anonymous requests — that is `Middleware.RequireAuth`.

```loft
enum AuthConfig {
    Jwt {
        secret:   text,       // HMAC-SHA256 secret key (keep in env var)
        issuer:   text,       // expected "iss" claim
        audience: text,       // expected "aud" claim; null = skip check
        leeway:   integer,    // clock skew tolerance in seconds (default 30)
        role_claim: text,     // JWT claim name containing roles (default "roles")
    },
    Session {
        store:    SessionStore,
        lifetime: integer,    // session lifetime in seconds (default 86400)
        cookie:   text,       // cookie name (default "sid")
    },
    ApiKey {
        header:   text,       // header name (default "X-API-Key")
        keys:     vector<ApiKeyEntry>,
    },
    Basic {
        realm:    text,
        users:    vector<BasicUser>,   // fixed credential list
    },
}

struct ApiKeyEntry {
    key:   text,            // the API key value
    owner: text,            // principal identity to assign
    roles: vector<text>,
}

struct BasicUser {
    username:      text,
    password_hash: text,    // Argon2id hash — never store plaintext
    roles:         vector<text>,
}

enum SessionStore {
    Memory,                 // in-process hash map; lost on restart
    File { path: text },    // one file per session in path/
}
```

### JWT authentication (loft decodes, native verifies)

The JWT is read from the `Authorization: Bearer <token>` header.  The
loft layer base64url-decodes the header and payload, parses the JSON claims,
and extracts `sub` (principal) and the `role_claim` field.  The native layer
performs HMAC-SHA256 signature verification and returns a boolean.

```
JWT flow (implemented across loft + native):
  1. loft: read Authorization header, strip "Bearer " prefix
  2. loft: base64url-decode header segment → parse JSON → verify alg == "HS256"
  3. loft: base64url-decode payload segment → parse JSON → extract sub, roles, exp
  4. loft: check exp > now and iss/aud claims
  5. native: verify HMAC-SHA256(header.payload, secret) == signature
  6. loft: set req.principal = sub, req.roles = roles
```

### Session authentication

Session IDs are random 128-bit values (generated by native, stored in loft's
session store).  The loft layer issues a `Set-Cookie` header on login and
reads the session cookie on subsequent requests.

```loft
// Issue a session (call from a login handler):
pub fn create_session(app: &App, principal: text, roles: vector<text>) -> Cookie

// Destroy a session (call from a logout handler):
pub fn destroy_session(app: &App, session_id: text) -> Cookie   // expire cookie
```

### Generating password hashes for BasicUser

```loft
// In a setup script, not in the server binary:
hash = argon2_hash("mysecretpassword");  // calls native Argon2id
println("password_hash: \"{hash}\"");
```

---

## Authorization

Authorization is checked after authentication.  Two mechanisms:

### Middleware-level guards

```loft
// All routes registered after this call require the given roles.
// Apply to an App before registering protected routes.
app.use_middleware(Middleware.RequireAuth);
app.use_middleware(Middleware.RequireRoles { roles: ["admin"] });
```

### Handler-level checks (in loft)

```loft
fn handle_delete_user(req: Request) -> Response {
    if !has_role(req, "admin") {
        return response_forbidden();
    }
    // ... delete logic ...
    response_no_content()
}
```

### Role-based route groups (pattern)

Because middleware is applied globally or not at all per `App`, fine-grained
per-route authorization is done with separate App instances sharing the same
`Server`:

```loft
fn main() {
    srv = Server { port: 8080 };

    // Public routes
    public = new_app(srv);
    get(public, "/", fn handle_home);
    get(public, "/health", fn handle_health);

    // Authenticated routes
    authed = new_app(srv);
    use_middleware(authed, Middleware.Authenticate { config: jwt_config });
    use_middleware(authed, Middleware.RequireAuth);
    get(authed, "/profile", fn handle_profile);
    post(authed, "/posts", fn handle_create_post);

    // Admin routes
    admin = new_app(srv);
    use_middleware(admin, Middleware.Authenticate { config: jwt_config });
    use_middleware(admin, Middleware.RequireRoles { roles: ["admin"] });
    delete(admin, "/users/{id}", fn handle_delete_user);

    serve_all([public, authed, admin]);
}
```

`serve_all` merges the route tables of multiple Apps and starts one server.

---

## Multi-threading model

The native layer manages a `tokio` async runtime with a thread pool sized to
`Server.threads` (default: CPU core count).  Each accepted connection is
handled within the async runtime.  When a route handler (a loft function)
needs to run, the native layer calls it on a dedicated blocking thread drawn
from a secondary blocking thread pool (Tokio's `spawn_blocking`).

This means:
- All loft handler code runs synchronously on blocking threads.
- I/O operations within handlers (database calls, outbound HTTP via `use web`)
  block the handler's thread, but do not block other connections.
- WebSocket handlers hold their thread for the lifetime of the connection.
- The number of concurrent WebSocket connections is bounded by the thread pool
  size; long-lived connections should be considered when sizing `threads`.

### Thread safety in middleware state

Rate-limit counters, session stores, and other shared middleware state are
stored in loft's parallel-safe store infrastructure (the same mechanism used
by `par(...)` loops).  The native layer serialises access at the store boundary.

---

## Static file serving

```loft
// Serve files from dir/ at /static/ URLs.
// GET /static/app.js → dir/app.js
// MIME types are derived from extensions in loft (no native needed).
serve_dir(app, "/static", "./public");

// Content-Type mapping is implemented in loft:
pub fn content_type_for(filename: text) -> text
// Returns "text/html", "application/javascript", "image/png", etc.
// Covers the 30 most common web extensions; falls back to "application/octet-stream".
```

---

## Complete example

```loft
use server;

struct LoginRequest { username: text, password: text }
struct LoginResponse { token: text }
struct Post { id: integer not null, title: text, body: text }

JWT_SECRET = "change-me-in-production";

fn handle_health(req: Request) -> Response {
    response_ok("ok")
}

fn handle_login(req: Request) -> Response {
    body = parse_json(req) as LoginRequest;
    if body == null {
        return response_bad_request("expected {\"username\":\"...\",\"password\":\"...\"}");
    }
    // In production: look up user, verify password hash.
    if body.username != "alice" or body.password != "secret" {
        return response_unauthorized();
    }
    token = jwt_sign(JWT_SECRET, body.username, ["user"], 3600);
    response_json("{LoginResponse { token: token }:j}")
}

fn handle_posts(req: Request) -> Response {
    // req.principal is populated by Authenticate middleware.
    posts = load_posts();
    response_json("{posts:j}")
}

fn handle_chat(req: Request, ws: &WebSocket) {
    user = req.principal ?? "anonymous";
    running = true;
    for _ in 0..1000000 if running {
        msg = ws_receive(ws);
        match msg {
            Text { content } =>
                ws_send(ws, WsMessage.Text { content: "{user}: {content}" }),
            Close { _, _ } => running = false,
            _ => {},
        }
    }
}

fn main() {
    jwt_config = AuthConfig.Jwt {
        secret:     JWT_SECRET,
        issuer:     "myapp",
        audience:   null,
        leeway:     30,
        role_claim: "roles",
    };

    srv = Server {
        port: 8443,
        tls: TlsConfig.Acme {
            config: AcmeConfig {
                domains: ["example.com"],
                email:   "admin@example.com",
                storage: "/var/lib/loft/certs/",
            },
        },
    };

    public = new_app(srv);
    use_middleware(public, Middleware.Logger);
    use_middleware(public, Middleware.SecureHeaders);
    get(public,  "/health", fn handle_health);
    post(public, "/login",  fn handle_login);
    serve_dir(public, "/", "./public");

    api = new_app(srv);
    use_middleware(api, Middleware.Logger);
    use_middleware(api, Middleware.Cors {
        origins: ["https://example.com"],
        methods: ["GET", "POST"],
        headers: ["Authorization", "Content-Type"],
        max_age: 86400,
    });
    use_middleware(api, Middleware.RateLimit { max_rps: 100, window: 60, by: "ip" });
    use_middleware(api, Middleware.Authenticate { config: jwt_config });
    use_middleware(api, Middleware.RequireAuth);
    get(api, "/api/posts", fn handle_posts);

    ws_app = new_app(srv);
    use_middleware(ws_app, Middleware.Authenticate { config: jwt_config });
    route_ws(ws_app, "/ws/chat", fn handle_chat);

    serve_all([public, api, ws_app])
}
```

---

## Native layer boundary

The native library (`libloft_server`) exposes these symbols to the loft
interpreter via `loft_register_v1`:

| Symbol | Purpose |
|--------|---------|
| `n_server_listen` | Bind TCP socket, start tokio runtime |
| `n_server_accept_loop` | Accept connections; call loft handler per request |
| `n_tls_load_pem` | Load PEM certificate + key into rustls config |
| `n_acme_provision` | Run ACME issuance flow; return cert bytes |
| `n_acme_renew_loop` | Background renewal task |
| `n_ws_send` | Write WebSocket frame |
| `n_ws_receive` | Read next WebSocket frame (blocking) |
| `n_ws_close` | Send close frame |
| `n_jwt_verify_hs256` | Verify HMAC-SHA256 signature |
| `n_argon2_hash` | Hash a password with Argon2id |
| `n_argon2_verify` | Verify a password against an Argon2id hash |
| `n_session_id_new` | Generate a cryptographically random session ID |
| `n_random_bytes` | Fill a buffer with random bytes (for CSRF tokens, etc.) |

Everything else — route matching, middleware execution, header parsing,
JWT claim extraction, session lookup, authorization checks, static file
MIME types — is implemented in the `.loft` source files and therefore
readable, testable, and modifiable in loft.

---

## Implementation phases

### Phase 1 — Plain HTTP server (no TLS)

- TCP listen + accept loop (native)
- HTTP/1.1 request parsing (native: hyper)
- `Request` / `Response` / `Header` structs in loft
- `new_app`, `route`, `get`/`post`/`put`/`delete`, `serve` in loft
- Route matching engine in loft
- `Middleware.Logger` in loft
- Response builder functions in loft
- Tests: routing, response codes, query params, path params

### Phase 2 — HTTPS with static certificates

- `rustls` integration in native layer
- `TlsConfig.Pem` support
- `Server.port` redirect (port 80 → 443) in loft
- `Middleware.SecureHeaders` in loft
- Tests: HTTPS handshake, certificate loading, redirect

### Phase 3 — WebSockets

- `tokio-tungstenite` in native layer
- `WebSocket` opaque struct, `ws_send`/`ws_receive`/`ws_close` in native
- `route_ws`, `WsMessage` enum in loft
- `Middleware.Authenticate` pre-handshake support
- Tests: echo server, message types, close handling

### Phase 4 — Authentication

- `n_jwt_verify_hs256`, `n_argon2_hash/verify`, `n_session_id_new` in native
- `AuthConfig` enum, `Middleware.Authenticate` in loft
- JWT decode pipeline (loft base64url decode + JSON parse + native verify)
- Session store (Memory + File) in loft
- `Middleware.RequireAuth`, `Middleware.RequireRoles` in loft
- `create_session`, `destroy_session`, login/logout pattern in loft
- Tests: JWT issuance + verification, session lifecycle, 401/403 responses

### Phase 5 — ACME / Let's Encrypt

- `instant-acme` integration in native layer
- `n_acme_provision`, `n_acme_renew_loop` in native
- `TlsConfig.Acme` config struct in loft
- HTTP-01 challenge handler wired into the port-80 listener
- Automatic renewal background loop
- Tests: staging environment, certificate storage, renewal trigger

### Phase 6 — Middleware + polish

- `Middleware.Cors` in loft
- `Middleware.RateLimit` in loft (using parallel store for thread safety)
- `Middleware.Decompress` in native (flate2)
- `serve_dir` + `content_type_for` in loft
- `parse_form`, `parse_multipart` in loft
- `serve_all` combining multiple Apps
- Performance testing: requests/second baseline on target hardware

---

## Dependencies

### Rust crates (native layer)

| Crate | Version | Purpose |
|-------|---------|---------|
| `tokio` | 1 | Async runtime, thread pool |
| `hyper` | 1 | HTTP/1.1 framing |
| `rustls` | 0.23 | TLS 1.2/1.3 |
| `tokio-tungstenite` | 0.24 | WebSocket |
| `instant-acme` | 0.7 | ACME RFC 8555 |
| `hmac` + `sha2` | 0.12 | JWT HMAC-SHA256 |
| `base64` | 0.22 | JWT base64url decode |
| `argon2` | 0.5 | Password hashing |
| `flate2` | 1 | gzip/deflate decompression |
| `rand` | 0.8 | Session ID generation |

### Loft dependencies (`loft.toml`)

```toml
[dependencies]
# None — server is a standalone package.
# The web package (HTTP client) is a separate library; both can be used
# together but neither depends on the other.
```

---

## Game server additions

When `server` is used to back a multiplayer game built with `game_client`, the
basic request/response design is insufficient.  This section documents the gaps
identified by evaluating the two libraries together and the additions that close
them.

---

### Gap 1 — JWT signing is missing

The complete example calls `jwt_sign(...)`, but the native boundary only
declares `n_jwt_verify_hs256`.  Game servers must issue tokens (on login or
`/join`) as well as verify them.

**Addition:** one new native symbol.

```
n_jwt_sign_hs256   alg, secret, sub, roles[], exp_secs → compact JWT text
```

The loft wrapper:

```loft
// Issue a signed JWT valid for exp_secs seconds.
// sub = principal identity; roles = authorization roles.
pub fn jwt_sign(secret: text, sub: text, roles: vector<text>, exp_secs: integer) -> text
```

---

### Gap 2 — No non-blocking WebSocket receive

Game servers run a tick loop that must drain messages from all player
connections since the last tick.  `ws_receive` blocks until a frame arrives —
it cannot be called per-player in a loop without deadlocking when some players
have sent nothing.

**Addition:** one new native symbol.

```
n_ws_poll   ws_handle → WsMessage | null
```

Returns the next queued frame without blocking, or `null` if the connection's
receive buffer is empty right now.

```loft
// Non-blocking receive.  Returns null if no frame is currently available.
pub fn ws_poll(ws: &WebSocket) -> WsMessage
```

A game tick loop uses `ws_poll` to drain every player's queue in order before
running the rules step.

---

### Gap 3 — No connection registry or broadcast

Each `route_ws` handler is isolated: it owns one `WebSocket` and has no
reference to any other connection.  A game server needs to send the same state
snapshot to every player in a lobby simultaneously.

**Two additions:**

#### 3a — ConnectionRegistry (implemented in loft)

A shared, thread-safe registry that maps a connection ID to its `WebSocket`
handle.  Implemented using loft's parallel-safe store (same infrastructure as
`RateLimit`).

```loft
struct ConnectionRegistry { /* opaque, backed by parallel store */ }

// Create the registry.  Passed to new_app() so middleware can populate it.
pub fn new_registry() -> ConnectionRegistry

// Register a connection (called at the start of every ws handler).
pub fn registry_add(reg: &ConnectionRegistry, id: text, ws: &WebSocket)

// Deregister when the handler exits.
pub fn registry_remove(reg: &ConnectionRegistry, id: text)

// Look up one connection's WebSocket.
pub fn registry_get(reg: &ConnectionRegistry, id: text) -> WebSocket

// Return all currently connected IDs.
pub fn registry_ids(reg: &ConnectionRegistry) -> vector<text>
```

#### 3b — `n_ws_broadcast` (native)

Sending to multiple connections from a single call avoids serializing messages
one at a time through the loft/native boundary for each recipient.

```
n_ws_broadcast   handles[], WsMessage → sent_count
```

```loft
// Send msg to every WebSocket in the list.  Returns number actually sent.
// Silently skips closed connections.
pub fn ws_broadcast(connections: vector<WebSocket>, msg: WsMessage) -> integer
```

Typical lobby broadcast:

```loft
fn broadcast_state(reg: &ConnectionRegistry, lobby_id: text, state: GameState) {
    ids   = registry_ids(reg);
    conns = [for conn_id in ids if starts_with(conn_id, "{lobby_id}/") {
        registry_get(reg, conn_id)
    }];
    payload = "{state:j}";
    env = GameEnvelope { type_id: MSG_STATE_FULL, seq: 0, tick: state.tick, payload: payload };
    ws_broadcast(conns, WsMessage.Text { content: "{env:j}" });
}
```

---

### Gap 4 — No server-side game loop

`serve` / `serve_all` models the server as pure request-response.  A game
server additionally needs a fixed-rate tick loop running independently of
incoming connections — collecting buffered inputs, stepping the rules WASM
module, and broadcasting the new state.

**Addition:** `game_loop.loft` — a new source file in `server/src/` providing
a server-side fixed-timestep loop.

```loft
struct GameLoop {
    tick_rate:      integer,   // ticks per second; default 20
    max_frame_time: integer,   // max ms per real frame before clamping; default 100
}

// Run a server-side tick loop until the process exits.
// Called alongside serve_all() on a separate thread.
// tick_fn(tick: integer) is called at the fixed rate.
pub fn run_game_loop(loop_cfg: GameLoop, tick_fn: fn(integer))
```

The native layer provides one symbol to support it:

```
n_sleep_until_us   target_us → void
```

`run_game_loop` is implemented in loft:

```loft
pub fn run_game_loop(loop_cfg: GameLoop, tick_fn: fn(integer)) {
    tick_us    = 1000000 / loop_cfg.tick_rate;
    max_us     = loop_cfg.max_frame_time * 1000l;
    next_us    = ticks();
    tick_count = 0;
    for _ in 0..2147483647 {
        now_us   = ticks();
        elapsed  = now_us - next_us;
        if elapsed > max_us { next_us = now_us; }   // spiral-of-death guard
        for _ in 0..1000 if ticks() >= next_us {
            tick_fn(tick_count);
            tick_count += 1;
            next_us    += tick_us as long;
        }
        sleep_until_us(next_us);
    }
}
```

Usage alongside `serve_all`:

```loft
fn game_tick(tick: integer) {
    // 1. drain each player's ws_poll queue into buffered inputs
    // 2. run rules WASM step
    // 3. broadcast state snapshot
}

fn main() {
    // ... set up Apps ...
    parallel {
        serve_all([public, ws_app]);
        run_game_loop(GameLoop { tick_rate: 20 }, fn game_tick);
    }
}
```

Until loft gets a `parallel {}` block, `run_game_loop` can be started on a
background thread via a native helper:

```
n_spawn_thread   fn() -> void → void
```

---

### Gap 5 — No WASM loading on server

The shared game logic pattern (see GAME_CLIENT_LIB.md § Shared game logic
pattern) requires both client and server to load and run the same `rules.wasm`
module.  The `server` library currently has no WASM runtime.

**Addition:** three new native symbols, identical to the client-side ones.

```
n_wasm_load        bytes: text (base64) → module_handle: integer
n_wasm_call        handle, export_name, args_json → result_json: text
n_wasm_has_export  handle, export_name → boolean
n_wasm_unload      handle → void
```

```loft
// Load a WASM module from base64-encoded bytes.  Returns an opaque handle.
pub fn wasm_load_bytes(bytes: text) -> integer

// Call a WASM export function.  args and result are JSON-encoded.
pub fn wasm_call(handle: integer, export: text, args: text) -> text

// True if the module exports the named symbol.
pub fn wasm_has_export(handle: integer, export: text) -> boolean

// Release a loaded WASM module.
pub fn wasm_unload(handle: integer)
```

The server-side WASM sandbox is identical to the client-side one:
only `loft::rand_u32()` and `loft::log()` are available as imports.
Ed25519 signature verification before loading is recommended (see
GAME_CLIENT_LIB.md § Security model).

---

### Gap 6 — `WsMessage` protocol divergence

`server` and `game_client` each define their own `WsMessage` enum with
subtly different variants (`Ping { data: text }` vs `Ping` with no fields).
Any shared protocol code that constructs or matches `WsMessage` values must
be duplicated with slight modifications.

**Resolution:** extract the shared protocol types into a lightweight
`game_protocol` package.

```
game_protocol/              ← GitHub: jjstwerff/loft-game-protocol
  loft.toml
  src/
    envelope.loft           ← GameEnvelope, MSG_* constants
    messages.loft           ← Msg* request/response structs
    ws_message.loft         ← canonical WsMessage enum
```

Both `server` and `game_client` declare this as a dependency:

```toml
# server/loft.toml
[dependencies]
game_protocol = "0.1"

# game_client/loft.toml
[dependencies]
game_protocol = "0.1"
```

The canonical `WsMessage` definition (in `game_protocol`):

```loft
enum WsMessage {
    Text   { content: text },
    Binary { data: text },     // raw bytes as base64
    Ping,
    Pong,
    Close  { code: integer, reason: text },
}
```

The server and game_client native layers both map their internal frame types
to this enum on receipt, and map from it on send.

---

### Gap 7 — Thread pool exhaustion for long-lived WebSocket connections

Each WebSocket handler holds a blocking thread for the entire lifetime of the
connection.  For a game server with 100 concurrent players, `Server.threads`
must be set large enough to accommodate all WebSocket handlers plus any
simultaneous HTTP request handlers.

**Guidance** (added to `server.loft` documentation):

```loft
// For game servers: set threads to at least max_concurrent_players + 10.
// Each WebSocket handler holds one thread; HTTP handlers need additional slots.
// Example: 64 players + 10 HTTP headroom = threads: 74
srv = Server {
    threads: 74,
    // ...
};
```

Long-term (deferred to Phase 7+): a connection manager model where WebSocket
handlers do not hold threads between frames, instead registering callbacks that
are invoked when frames arrive.  This scales to thousands of concurrent
connections with a small thread pool.  Requires async loft handler support,
which depends on the coroutine design (COROUTINE.md).

---

### Summary of additions

| # | What | Where | Type |
|---|------|-------|------|
| 1 | `n_jwt_sign_hs256` | native layer | new native symbol |
| 2 | `n_ws_poll` | native layer | new native symbol |
| 3a | `ConnectionRegistry` + accessors | `websocket.loft` | loft |
| 3b | `n_ws_broadcast` | native layer | new native symbol |
| 4a | `GameLoop`, `run_game_loop` | `game_loop.loft` (new file) | loft |
| 4b | `n_sleep_until_us`, `n_spawn_thread` | native layer | new native symbols |
| 5 | `n_wasm_load/call/has_export/unload` | native layer | new native symbols (4) |
| 6 | `game_protocol` shared package | new repo | loft package |
| 7 | Thread-sizing guidance | `server.loft` docs | documentation |

Updated native symbol table (additions marked **new**):

| Symbol | Purpose |
|--------|---------|
| `n_server_listen` | Bind TCP socket, start tokio runtime |
| `n_server_accept_loop` | Accept connections; call loft handler per request |
| `n_tls_load_pem` | Load PEM certificate + key into rustls config |
| `n_acme_provision` | Run ACME issuance flow; return cert bytes |
| `n_acme_renew_loop` | Background renewal task |
| `n_ws_send` | Write WebSocket frame |
| `n_ws_receive` | Read next WebSocket frame (blocking) |
| `n_ws_poll` | **new** — non-blocking receive; null if empty |
| `n_ws_broadcast` | **new** — send to multiple connections |
| `n_ws_close` | Send close frame |
| `n_jwt_verify_hs256` | Verify HMAC-SHA256 signature |
| `n_jwt_sign_hs256` | **new** — sign a JWT (issue tokens) |
| `n_argon2_hash` | Hash a password with Argon2id |
| `n_argon2_verify` | Verify a password against an Argon2id hash |
| `n_session_id_new` | Generate a cryptographically random session ID |
| `n_random_bytes` | Fill a buffer with random bytes |
| `n_wasm_load` | **new** — load WASM module bytes → handle |
| `n_wasm_call` | **new** — call WASM export function |
| `n_wasm_has_export` | **new** — check WASM export exists |
| `n_wasm_unload` | **new** — release WASM module |
| `n_sleep_until_us` | **new** — sleep until absolute microsecond timestamp |
| `n_spawn_thread` | **new** — run a loft fn on a background thread |

Total: 13 original symbols + 10 new = **23 native symbols**.
