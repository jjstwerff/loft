<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 02 — Bridge intelligence: warm pool, multiplex, cache, recovery

**Status:** Open (depends on phase 01)

## Goal

Turn the bridge from "JSON proxy that happens to be in Rust"
into the LAYER that justifies a Rust sidecar.  Seven
capabilities, each independently load-bearing for the "done
right" quality bar in the plan-14 README.

By the end of phase 02, killing rust-analyzer, restarting the
viewer, opening four browser tabs, or typing 30 chars per
second don't break the experience.  Each shipping criterion
is testable in isolation.

## What ships

### 02.1 — Server warm pool

Bridge keeps spawned LSP servers alive across viewer
disconnects.  Server state:

```rust
// tools/loft-lsp-bridge/src/pool.rs (NEW)
pub struct ServerPool {
    servers: HashMap<(Language, WorkspaceRoot), PooledServer>,
}

struct PooledServer {
    server: Box<dyn LanguageServer>,   // RustAnalyzerServer / LoftLspServer / JdtlsServer
    last_used: Instant,
    open_documents: HashMap<Url, Arc<DocumentState>>,
    clients: Vec<ClientId>,            // who currently has it open
}
```

Lifecycle:

- **Spawn**: first viewer-side `lsp.open` for a workspace
  triggers a server spawn.
- **Reuse**: subsequent `lsp.open` requests in the same
  workspace + language reuse the live server.
- **Idle TTL**: after 30 min with zero connected clients, the
  server is shut down (`shutdown` request, then `exit`).
  TTL is configurable via `LOFT_LSP_BRIDGE_IDLE_TTL_MIN`.
- **Eager keepalive**: bridge sends a no-op (`workspace/didChangeConfiguration`
  with empty settings) every 5 min so the server doesn't go idle on its own.

Acceptance: `make view`, open a `.rs` file (cold-start ~30 s),
`Ctrl+C` the viewer, restart `make view`, open the same file
→ first hover ≤ 200 ms.

### 02.2 — Multi-client multiplex

Multiple browser tabs share one server per workspace+language:

```rust
// tools/loft-lsp-bridge/src/multiplex.rs (NEW)
pub struct ServerMultiplexer {
    server: PooledServer,
    next_request_id: u64,
    /// Maps (server_request_id) → (client_id, original_client_request_id)
    in_flight: HashMap<u64, (ClientId, u64)>,
}

impl ServerMultiplexer {
    pub async fn forward(&mut self, client_id: ClientId, req: ClientRequest) -> ServerRequest { ... }
    pub async fn return_response(&mut self, server_resp: ServerResponse) -> Option<ClientResponse> { ... }
}
```

Each client has its own request ID space; the bridge owns
the SERVER-side ID space and rewrites IDs in both directions.
Cancellations track per client (cancelling tab 1's hover
doesn't cancel tab 2's).

Acceptance: open the same file in two browser tabs; trigger
hover in both simultaneously; both responses arrive at the
right tab; bridge log shows ONE rust-analyzer request per
unique (URI, position), not two.

### 02.3 — Per-document state cache

Bridge caches per-document state so cancelled / re-issued
requests can be answered locally:

```rust
struct DocumentState {
    uri: Url,
    text: String,                       // current text (for re-sync)
    version: i32,                       // LSP textDocument version
    offsets: LineOffsets,               // precomputed line→byte map for position translation
    last_diagnostics: Vec<Diagnostic>,  // for late-joining clients
    semantic_tokens: Option<SemanticTokens>,  // last full snapshot for token-delta clients
}
```

When a new client connects to a workspace with already-open
documents, the bridge ships the cached state immediately
(synthetic `publishDiagnostics`, cached semantic tokens) so
the new tab doesn't see an empty editor while the server
re-derives.

Acceptance: open file in tab 1, wait for diagnostics; open
tab 2 → diagnostics visible immediately, no server
round-trip.

### 02.4 — Debounce + backpressure

Viewer's `didChange` events fire on every keystroke (phase 05
when editing lands).  Bridge collapses bursts before
forwarding:

```rust
// tools/loft-lsp-bridge/src/debounce.rs (NEW)
pub struct DebounceQueue {
    pending: HashMap<Url, PendingChange>,
    flush_after: Duration,  // default 200 ms
}
```

If the same URI gets multiple `didChange` events within
200 ms, only the last one is forwarded.  Hover requests
queued during a burst are held until the document settles
(then re-issued against the new version).

Backpressure: if the SERVER queue grows past 100 in-flight
requests, the bridge starts rejecting non-essential client
requests with a "server overloaded; retry shortly" error
rather than letting the queue grow unboundedly.

Acceptance: synthetic test sends 100 `didChange` events in
1 s; server receives ≤ 5 (one per ~200 ms window) plus the
final snapshot.

### 02.5 — Crash recovery

Bridge detects when an LSP server dies and respawns
automatically.  State replay:

1. Server child process exits (detected via `tokio::process`
   wait future).
2. Bridge logs the crash with the last 100 stderr lines.
3. Bridge respawns the server at the same workspace.
4. Bridge replays `initialize` + every cached open-document's
   `didOpen` against the new instance.
5. Connected clients see ONE `bridge.server_restarted`
   notification (carrying the restart reason); their open
   documents are still valid.

Acceptance: `make view`, open a file, hover works; from another
shell `kill -9 $(pgrep rust-analyzer)`; trigger another hover
in the browser → response arrives within 5 s; banner shows
"rust-analyzer restarted (signal: KILL)".  No hard refresh
required.

### 02.6 — Structured tracing surface

Each request gets a `tracing` span with method, URI, position,
client ID, server ID, and durations for each pipeline stage.
Bridge exposes a `/lsp_bridge/logs/<session_id>` endpoint
that streams the current session's log via Server-Sent
Events.

Viewer footer: a "View LSP logs" link that opens this endpoint
in a side panel.  Filter by method / URI / time range.

Acceptance: trigger a hover, open the logs panel, see the
hover request as a span with sub-ms timings for protocol
encode + server forward + server response + decode.

### 02.7 — Schema translation: bridge dialect vs LSP dialect

Bridge speaks one normalised JSON-RPC dialect to the viewer;
LSP-server-specific quirks hidden behind it:

- rust-analyzer returns hover content as `MarkupContent`
  with `kind: markdown`; viewer doesn't care about the kind,
  just renders `.value`.  Bridge unwraps.
- jdtls returns positions in UTF-16 even if the client asked
  for UTF-8; bridge re-translates.
- loft-lsp (eventually) returns `loft://`-prefixed URIs for
  generated stdlib symbols; bridge rewrites to `/file/...`
  paths the viewer can navigate.

Each translation lives in its own module with unit tests.
Adding a new server (phase 03 / 04) means adding a translator
module, not patching the viewer.

Acceptance: bridge has a `Translator` trait per language;
each implementation has its own test suite; viewer-side code
is server-agnostic.

## Acceptance — phase 02 as a whole

1. **Warm pool**: viewer restart hits server in ≤ 200 ms.
2. **Multiplex**: two tabs share one rust-analyzer; per-tab
   request IDs don't collide.
3. **Cache**: late-joining tab gets diagnostics immediately.
4. **Debounce**: 100 `didChange` events in 1 s collapse to
   ≤ 5 server forwards.
5. **Recovery**: `kill -9 $(pgrep rust-analyzer)` invisible to
   the user beyond a brief banner.
6. **Tracing**: "View LSP logs" panel shows per-request
   spans with stage timings.
7. **Schema translation**: server-specific quirks hidden
   behind the `Translator` trait; viewer code knows nothing
   about server dialects.
8. CI: `tests/lsp_bridge_pool.rs`, `tests/lsp_bridge_multiplex.rs`,
   `tests/lsp_bridge_recovery.rs`, `tests/lsp_bridge_debounce.rs`
   all pass.
9. Latency budget: hover P95 ≤ 50 ms (warm); cold start ≤
   2 s for previously-indexed workspaces.

## Risks

| Risk | Mitigation |
|---|---|
| Warm pool memory grows unbounded with many workspaces | TTL + max-pool-size (5 servers); LRU eviction.  Configurable via env vars. |
| Multiplex ID rewriting introduces races | All ID maps live behind a single `tokio::sync::Mutex`; per-client request map updated atomically.  Pinned by stress test (1000 concurrent requests). |
| Cache divergence between bridge view and server view | Bridge uses LSP `version` field to detect stale state; on mismatch, requests a full re-sync via `textDocument/didChange` with the cached text. |
| Crash recovery loops if the server crashes on every replay | Backoff: 1 s, 2 s, 5 s, 10 s, 30 s, then GIVE UP and surface a fatal error to the viewer.  Log preserves the full crash chain. |
| Debounce hides keystroke responsiveness | 200 ms default tuned to match human "did I make a typo" perception threshold.  Configurable via env var; can be 0 for testing. |
| Tracing logs balloon in long sessions | Logs roll at 100 MB / 1 hour, whichever first.  Old segments compressed to `.zst`. |
| Schema translation introduces server-specific code paths the bridge claims to hide | Each `Translator` impl has a "strict mode" flag in tests that asserts no server-specific concept escapes into the viewer dialect.  CI gate. |

## Critical files

| Path | Action |
|---|---|
| `tools/loft-lsp-bridge/src/pool.rs` | NEW — warm pool + idle TTL |
| `tools/loft-lsp-bridge/src/multiplex.rs` | NEW — per-client ID rewriting |
| `tools/loft-lsp-bridge/src/cache.rs` | NEW — per-document state cache |
| `tools/loft-lsp-bridge/src/debounce.rs` | NEW — `didChange` collapse + backpressure |
| `tools/loft-lsp-bridge/src/recovery.rs` | NEW — server-crash detect + respawn + replay |
| `tools/loft-lsp-bridge/src/translator.rs` | NEW — `Translator` trait + per-server impls |
| `tools/loft-lsp-bridge/src/routing.rs` | EXTEND — wire all of the above into request dispatch |
| `tools/viewer/src/main.loft` | EXTEND — footer "View LSP logs" link; banner on `bridge.server_restarted` notification |
| `tools/viewer/static/lsp_logs_panel.js` | NEW — SSE-streamed log viewer panel |
| `tests/lsp_bridge_pool.rs` | NEW — warm-pool acceptance |
| `tests/lsp_bridge_multiplex.rs` | NEW — two-client request stream |
| `tests/lsp_bridge_recovery.rs` | NEW — kill rust-analyzer, assert auto-recover |
| `tests/lsp_bridge_debounce.rs` | NEW — burst collapse |
| `tests/lsp_bridge_translator.rs` | NEW — per-server dialect-translation roundtrip |

## What phase 02 does NOT ship

- Loft-lsp integration — phase 03.
- Java/jdtls integration — phase 04.
- Browser-side editing surface — phase 05.
- Workspace-symbol search (phase 05 / E3).
- Code-actions / quick-fixes (phase 05 / E3).

## Cross-references

- [Phase 00 — scaffold](00-scaffold.md), [Phase 01 — rust-analyzer](01-rust-analyzer.md)
  — phase 02 layers on top of these.
- [Plan-14 README — What the bridge DOES](README.md#what-the-bridge-does-the-differentiator)
  — the seven capabilities listed there map 1:1 to the
  02.1–02.7 sections in this phase doc.
- [`tracing` patterns](https://docs.rs/tracing/) — span-based
  request tracing.
- [`tokio::sync` primitives](https://docs.rs/tokio/latest/tokio/sync/)
  — Mutex / RwLock / mpsc channels used by the multiplexer.
