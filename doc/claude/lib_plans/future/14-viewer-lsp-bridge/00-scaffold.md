<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 00 — Scaffold the bridge binary + Layer-B protocol

**Status:** Open

## Goal

Stand up the `loft-lsp-bridge` Rust binary, define the Layer-B
(viewer ↔ bridge) IPC protocol, and prove the round-trip with
an echo server.  No LSP servers spawned yet — phase 00 is
purely about the architectural backbone the rest of the plan
sits on.

By the end of this phase, the viewer can connect to the
bridge over a Unix domain socket, send a JSON message, and
receive a structured reply.  Nothing else.  But every later
phase rests on this protocol contract being right.

## What ships

### `tools/loft-lsp-bridge/` Rust binary

A new workspace crate at `tools/loft-lsp-bridge/`:

```
tools/loft-lsp-bridge/
├── Cargo.toml           # binary crate; deps: tokio, tokio-util, interprocess,
│                        #                    serde, serde_json, lsp-types,
│                        #                    lsp-server, tracing, anyhow
├── README.md            # usage + design summary (links back to plan-14)
└── src/
    ├── main.rs          # CLI entry: parse args, bind socket, run accept loop
    ├── transport.rs     # Layer-B transport: framed JSON over Unix socket
    ├── protocol.rs      # Message types: Request, Response, Notification
    ├── routing.rs       # Per-connection state, request dispatcher (stub for phase 00)
    └── tracing_init.rs  # Structured logging setup (subscriber, format, env filter)
```

CLI surface:

```bash
loft-lsp-bridge                          # default socket path: $XDG_RUNTIME_DIR/loft-lsp-bridge.sock
loft-lsp-bridge --socket /tmp/foo.sock   # custom path
loft-lsp-bridge --foreground             # don't daemonize (default; daemonize is a stretch)
loft-lsp-bridge --log-level debug        # tracing filter
loft-lsp-bridge --version
loft-lsp-bridge --help
```

Cross-platform via `interprocess` crate: Unix domain socket on
Linux/macOS, named pipe on Windows.  Same `LocalSocketStream`
abstraction.

### Layer-B protocol — length-prefixed JSON over Unix socket

Wire format:

```
┌──────────────┬───────────────────────────────────┐
│ u32 LE len   │  <len> bytes of UTF-8 JSON        │
└──────────────┴───────────────────────────────────┘
```

Length prefix is a 4-byte little-endian unsigned integer.
JSON payload is one of three shapes:

```rust
// src/protocol.rs
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Frame {
    Request(Request),
    Response(Response),
    Notification(Notification),
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Request {
    pub id: u64,                // viewer-assigned; bridge echoes back in Response
    pub method: String,         // e.g. "echo", later "lsp.hover", "lsp.definition"
    pub params: serde_json::Value,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Response {
    pub id: u64,
    #[serde(flatten)]
    pub result: ResponseResult,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(untagged)]
pub enum ResponseResult {
    Ok { result: serde_json::Value },
    Err { error: BridgeError },
}

#[derive(Serialize, Deserialize, Debug)]
pub struct BridgeError {
    pub code: i32,              // mirrors LSP error codes where applicable
    pub message: String,
    pub data: Option<serde_json::Value>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Notification {
    pub method: String,
    pub params: serde_json::Value,
}
```

Why length-prefix instead of `Content-Length:` framing (LSP's
own spec)?

- **Cleaner**: no header parsing, no `\r\n\r\n` boundary
  ambiguity.  Just a u32 + bytes.
- **No protocol carry-over**: the Layer-B protocol stays our
  own; we don't accidentally inherit LSP's framing quirks.
- **Tokio idiom**: `tokio_util::codec::LengthDelimitedCodec`
  ships exactly this framing out of the box.

Why JSON not Cap'n Proto / Protobuf?

- **Bridge proxies LSP messages mostly unchanged**.  Layer-B
  carries the same JSON the bridge will turn around and send
  to rust-analyzer (Layer C).  Re-serialising into a different
  schema would be wasted work.
- **Debuggability**: `tracing` log lines stay human-readable;
  any colleague can `tail -f /tmp/loft-lsp-bridge.log` and see
  what's happening.
- **Schema evolution**: adding fields is non-breaking; we're
  not paying serialisation overhead the bridge would notice.

### `lib/lsp_bridge_client/` loft library

The viewer-side wrapper that hides the socket protocol behind
a clean loft API:

```loft
// lib/lsp_bridge_client/src/lsp_bridge.loft
pub struct BridgeClient { /* opaque handle */ }

pub fn lsp_bridge_connect(socket_path: text) -> BridgeClient;
#impure(host_io)

pub fn lsp_bridge_request(self: BridgeClient, method: text, params: text) -> text;
//                                                              ^ JSON ^ JSON
#impure(host_io)

pub fn lsp_bridge_close(self: BridgeClient);
#impure(host_io)
```

For phase 00, params + result are passed as JSON text.  Once
the loft JSON library (default/06_json.loft) is comfortable
with vector<struct> destructuring (Q1 in QUALITY.md), the API
gets typed wrappers.

### Echo round-trip

Phase 00 ships ONE method: `echo`.

```loft
// In the viewer, somewhere visible during testing:
client = lsp_bridge_connect("/tmp/loft-lsp-bridge.sock");
reply = client.lsp_bridge_request("echo", "\"hello\"");
println(reply);  // → "hello"
```

This validates:
- Socket bind + connect work cross-process.
- Length-prefixed framing is correctly implemented in BOTH
  Rust (bridge) and loft (client lib).
- Tokio runtime + `LocalSocketStream` interop with loft's
  blocking I/O (loft calls into the Rust client lib, which
  blocks on the read).
- `serde_json` round-trip preserves the payload.

### Tracing infrastructure

```rust
// tools/loft-lsp-bridge/src/tracing_init.rs
pub fn init() -> WorkerGuard {
    let (writer, guard) = tracing_appender::non_blocking(
        tracing_appender::rolling::never("/tmp", format!("loft-lsp-bridge-{}.log", std::process::id()))
    );
    tracing_subscriber::fmt()
        .with_writer(writer)
        .with_env_filter(EnvFilter::from_default_env().add_directive("info".parse().unwrap()))
        .with_target(true)
        .init();
    guard
}
```

Each request gets a span:

```rust
#[tracing::instrument(skip(state), fields(id = req.id, method = %req.method))]
async fn handle_request(state: &mut RouterState, req: Request) -> Response { ... }
```

Logs go to `/tmp/loft-lsp-bridge-<pid>.log`.  Phase 02 adds
the "View LSP logs" footer link in the viewer; phase 00 just
ensures the file is being written.

## Critical files

| Path | Action |
|---|---|
| `Cargo.toml` (workspace root) | ADD `tools/loft-lsp-bridge` to `[workspace] members` |
| `tools/loft-lsp-bridge/Cargo.toml` | NEW — bin crate + dep list |
| `tools/loft-lsp-bridge/src/main.rs` | NEW — CLI entry + accept loop |
| `tools/loft-lsp-bridge/src/transport.rs` | NEW — `LengthDelimitedCodec`-wrapped `LocalSocketStream` |
| `tools/loft-lsp-bridge/src/protocol.rs` | NEW — `Frame`, `Request`, `Response`, `Notification` types |
| `tools/loft-lsp-bridge/src/routing.rs` | NEW — phase-00 stub: only the `echo` method |
| `tools/loft-lsp-bridge/src/tracing_init.rs` | NEW — tracing subscriber setup |
| `lib/lsp_bridge_client/loft.toml` | NEW — package manifest |
| `lib/lsp_bridge_client/src/lsp_bridge.loft` | NEW — loft-side API + native fn declarations |
| `src/native/lsp_bridge.rs` (or wherever native fns live) | NEW — Rust impl of the `n_lsp_bridge_*` natives that wrap `std::os::unix::net::UnixStream` (or `interprocess::local_socket::LocalSocketStream` for cross-platform) |
| `Makefile` | ADD `lsp-bridge-build` target — `cargo build --release -p loft-lsp-bridge`; `make view` checks for the binary at startup |
| `tests/lsp_bridge_echo.rs` | NEW — Rust integration test: spawn bridge, connect, send echo, assert response |
| `tests/scripts/lsp_bridge_echo.loft` | NEW — loft integration test: round-trip via the loft-side API |

## Cargo dependencies for `loft-lsp-bridge`

```toml
[package]
name = "loft-lsp-bridge"
version = "0.1.0"
edition = "2024"

[dependencies]
tokio = { version = "1", features = ["full"] }
tokio-util = { version = "0.7", features = ["codec"] }
interprocess = { version = "2", features = ["tokio"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
lsp-types = "0.97"
lsp-server = "0.7"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
tracing-appender = "0.2"
anyhow = "1"
clap = { version = "4", features = ["derive"] }
```

Per CLAUDE.md dependency policy: each new dep is justified
in `tools/loft-lsp-bridge/README.md`.  None of the above are
exotic — all standard Rust ecosystem.

## Acceptance

1. `cargo build --release -p loft-lsp-bridge` produces
   `target/release/loft-lsp-bridge`.
2. `cargo test -p loft-lsp-bridge` passes (unit tests for
   framing, JSON round-trip).
3. `tests/lsp_bridge_echo.rs` passes: spawns the bridge,
   connects via `interprocess::local_socket`, sends a length-
   prefixed `{"kind":"request","id":1,"method":"echo","params":"hello"}`,
   reads back `{"kind":"response","id":1,"result":"hello"}`.
4. `tests/scripts/lsp_bridge_echo.loft` passes: same shape
   but exercised through the loft-side `lib/lsp_bridge_client`
   wrapper.
5. Bridge writes a per-PID log to `/tmp/loft-lsp-bridge-<pid>.log`
   with structured request spans.
6. Bridge survives `Ctrl+C` cleanly: closes all client
   connections, removes the socket file, exits with status 0.
7. `make view` checks for the bridge binary at startup; if
   missing, prints a clear "run `make lsp-bridge-build`"
   message and falls back to the existing read-only viewer
   (graceful degradation).
8. CI gate: `cargo fmt --check`, `cargo clippy --all-targets
   -- -D warnings`, `cargo build --no-default-features` all
   pass.

## Risks

| Risk | Mitigation |
|---|---|
| `interprocess` crate has rough edges on Windows | Phase 00 acceptance includes a Windows CI lane.  If Windows support is wonky, phase 00 ships Unix-only and a follow-up issue tracks Windows. |
| `tokio` async runtime adds startup latency the viewer notices | Bridge is a long-lived daemon; startup runs once.  Viewer connects to existing socket → cost amortised. |
| Loft's blocking I/O calls into the Rust client lib will deadlock with `tokio` | Client lib uses BLOCKING `std::os::unix::net::UnixStream`, not tokio.  Bridge SERVER uses tokio for fan-out across multiple clients; client SIDE is blocking + simple. |
| Length-prefix framing diverges between Rust and loft (endianness, signed vs unsigned) | Pin a fixed test corpus in `tests/lsp_bridge_echo.rs`: known-byte-sequence in, known-byte-sequence out.  Both Rust + loft sides assert against the same hex bytes. |
| Bridge binary not on `$PATH` for the viewer to find | `make view` looks at `target/release/loft-lsp-bridge` first, then `~/bin/loft-lsp-bridge` (plan-37 phase 08 install location), then `$PATH`.  Clear error if none found. |
| Logs leak to `/tmp` filling the disk | Logs roll per-PID; bridge's `Drop` cleans up the log file when the process exits.  Stretch: rotate to `/tmp/loft-lsp-bridge/` and cap retention. |

## What phase 00 explicitly does NOT ship

- LSP server spawning (phase 01).
- Any LSP-protocol-aware methods beyond `echo` (phase 01).
- Browser-side UI changes (phase 05).
- Bridge intelligence (warm pool, multiplex, cache) — phase 02.

The point is the protocol contract.  Everything else stacks
on top of a working contract; nothing stacks on top of
unverified guesses.

## Cross-references

- [Plan-14 README — Architecture](README.md#architecture) —
  the three IPC layers; phase 00 covers Layer B end-to-end.
- [Plan-37 phase 07 — loft-native scanner](../../../plans/37-tracker-index/07-loft-native-scanner.md)
  — sibling daemon-binary precedent (`loft-index`), same
  install + lifecycle pattern.
- [`interprocess` docs](https://docs.rs/interprocess/) —
  the cross-platform local-socket abstraction.
- [`tokio_util::codec::LengthDelimitedCodec`](https://docs.rs/tokio-util/latest/tokio_util/codec/struct.LengthDelimitedCodec.html)
  — the framing primitive.
- [`tracing` docs](https://docs.rs/tracing/) — structured
  logging the bridge uses for the "View LSP logs" feature.
