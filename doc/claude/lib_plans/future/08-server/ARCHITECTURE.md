<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 08 — HTTP server: core I/O + concurrency architecture

The design that the rest of [README.md](README.md) builds on — how requests
actually flow between native I/O and loft logic, and how it scales. Resolves the
open architecture fork (the README's hyper/tokio callback model vs. the shipped
polled model) against the present interpreter, via the
[design-protocol](../../../../../.claude/skills/design-protocol/SKILL.md): one
invariant, the load-bearing claim falsified before committing.

> **Supersedes** the README's "native event loop calls a loft handler per
> request" + "thread-pool bridging tokio to blocking loft handlers" assumption —
> that model is **not buildable** on loft's FFI (see below). The README's
> loft-side design (types, routing, middleware, auth, TLS via native primitives)
> is unchanged; only the I/O/concurrency core is specified here.

---

## The invariant (one sentence)

> The native layer owns **I/O and readiness** (a `mio` reactor: accept, per-
> socket buffered read/write, the ready-set); **loft owns logic and *drives***
> by draining ready events and invoking handlers — so the runtime boundary is
> **only ever loft→native** (poll / read / write / respond), **never
> native→loft**.

## The load-bearing claim — probed, the callback model is rejected

The README assumes native can call a loft handler. Probed against the code; it
**cannot**, and nothing cheap changes that:

- **The FFI is one-directional.** A native function is `fn(&mut Stores, &mut
  DbRef)` (`src/database/mod.rs:52`) — it receives the store + a stack pointer,
  **no `State` handle**, so it cannot run interpreter bytecode. `loft-ffi`
  exposes only store-allocation callbacks, no function-invoke path.
- **It is enforced, not incidental.** `src/native_gate.rs:23-27`: *"a native
  function only calls native functions — the runtime boundary is only ever
  interpret→native … never the hard native→interpreter upcall."* The native
  backend is a closed callee-DAG; runtime fn-refs (`CallRef`) are excluded
  precisely because they could point at interpreted code.
- **`State` is `!Send`** (`src/state/mod.rs`) — one per OS thread — so even a
  hypothetical upcall couldn't run a loft handler on a tokio worker without
  per-thread interpreters + a dispatch queue.

So "tokio accepts → calls the loft handler" would need: a new FFI re-entry
type + relaxing the native gate + a thread-safe handler-dispatch queue to a
single interpreter thread. That is a language-runtime project, not a library.
**Rejected.** The boundary stays loft→native, which the FFI already makes
**loud by construction** — you cannot write the wrong-direction call.

## The architecture: native reactor, loft drains it

This is the **`engine_host` pattern**, already proven in-tree
(`lib/engine_host/src/engine_host.loft:238` — `run()` loops `kernel_pump()` +
`kernel_next_event()` and loft invokes the `on_event` closures). Apply it to
HTTP:

```
                native (libloft_server)                    loft (server lib)
   ┌──────────────────────────────────────────┐    ┌───────────────────────────┐
   │  mio reactor (epoll / kqueue / IOCP)      │    │  serve(app):              │
   │   • accept() → queue new conns            │    │    while running {         │
   │   • per-socket readiness + read/write bufs│◀───│      n = reactor_poll(ms) │ drain
   │   • HTTP/1.1 + WS framing (hyper-light /   │───▶│      for ev in 0..n {     │ ready
   │     tungstenite)                          │    │        req = take_request()│ events
   │   • TLS termination (rustls)              │    │        resp = dispatch(app,│
   │  EXPOSES poll/take/respond primitives —   │    │               req)         │
   │  NEVER calls loft.                        │    │        respond(req, resp) │
   └──────────────────────────────────────────┘    │      } } }                 │
                                                    └───────────────────────────┘
```

- **Native** runs the `mio` event loop on its own thread(s), accepts promptly
  into a bounded queue, does the byte-level read/write/framing/TLS, and maintains
  the **ready-set**. It exposes primitives only: `n_reactor_poll(timeout) ->
  count`, `n_reactor_take_request() -> handle`, request accessors
  (`n_req_method/path/header/body`), `n_respond(handle, status, body)`,
  `n_ws_take/send/close`. No symbol calls loft.
- **Loft** owns `serve(app)`: a single drain loop that calls `reactor_poll`
  once per tick (returns only the count of *ready* events — **O(ready), not
  O(N)**), then for each ready event takes the request, runs the loft
  `dispatch` (route match → middleware pipeline → handler — all the README's
  loft-side logic, unchanged), and calls `respond`. The public API
  (`new_app`/`route`/`get`/`serve`) is identical to the README; only `serve`'s
  body changes from per-client polling to reactor draining.

## Why this fixes the measured connect-storm (the SRV driver)

The @PLN6 load test measured connect latency degrading super-linearly
(0.6→7.3 s/client at 30 clients) because the **single loft loop** did
`accept()` + **O(N) per-client `recv` polling** + O(cells) replay + snapshot —
so `accept()` was starved by the polling. This architecture removes both
loft-side costs:

- **Accept is native + prompt.** `mio` accepts into a queue regardless of how
  fast loft drains — connect latency decouples from dispatch rate and stays
  flat under a storm.
- **No O(N) polling.** Loft drains only the **ready** set (`reactor_poll`
  returns ready count), so an idle-heavy server costs O(ready) per tick, not
  O(clients). (This subsumes today's `pump-fast-idle-poll` fix — the 20 ms
  per-idle-client timeout disappears because idle sockets are simply not in the
  ready-set.)
- Replay-on-connect is **app-level** (Tier A: chunked replay) — not the
  server's job; the server just makes accept + readiness cheap.

So the "Tier-B reactor" is **not a later phase** — it *is* this core
architecture, and it should be built **first**, as the foundation the README's
HTTP/TLS/WS/auth phases sit on.

## Concurrency

- **Dispatch is single-threaded** in loft (one drain loop). For an I/O-bound
  server this is right — the bytes move in native; loft only routes + runs
  handler logic, which is fast. No `State`-sharing problem.
- **Heavy per-request compute** (rare) can use `par` inside a handler — rayon,
  each worker a deep-copied store snapshot (`clone_for_worker`,
  `src/database/allocation.rs:786`). Not the default path.
- **Shared mutable state** (sessions, rate-limit counters) lives **native-side**
  behind the reactor (a Rust map under the reactor's lock), exposed to loft as
  get/set primitives — *not* in a loft store, because loft's parallel model
  deep-copies + read-locks inputs (it has no shared-mutable-across-threads
  store). Single-threaded dispatch means even a loft-side map is safe, but
  native-side keeps it correct if dispatch ever shards.

## What changes vs. the README

| README assumed | This architecture |
|---|---|
| `n_server_accept_loop` *calls loft handler* | `n_reactor_poll` / `n_reactor_take_request` — loft drains; native never calls loft |
| tokio thread-pool bridging to blocking loft handlers | single loft drain loop; `par` only for heavy handler compute |
| hyper/tokio async owns the flow | `mio` reactor owns I/O; loft owns flow (the `engine_host` pattern) |
| Tier-B reactor is a deferred multi-threading phase | the reactor **is** the core; built first as the foundation |

Everything else in the README — `Request`/`Response`/`Middleware` types, route
matching, the middleware pipeline, JWT/session/auth logic, TLS config surface —
is loft-side or native-primitive and **carries over unchanged**.

## Open items

- **macOS kqueue validation.** `mio` abstracts epoll/kqueue/IOCP, but the
  kqueue path needs a real macOS run (the OS component most likely to diverge) —
  the one hardware dependency.
- **Backpressure.** The accept queue + per-socket write buffers are bounded;
  define the over-limit policy (drop / 503 / close) — a native-side choice.
- **Graceful shutdown / draining** the in-flight ready-set on `serve` exit.

## See also

- [README.md](README.md) — the full server design (types, routing, phases).
- `lib/engine_host/src/engine_host.loft` — the proven native-pump / loft-drive
  precedent this generalises.
- `doc/claude/THREADING.md` — the `par` store-isolation model handlers reuse.
