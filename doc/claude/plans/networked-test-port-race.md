# The networked-test "flakes" are one port race — and a real `server::listen` bug

> **Status: harness FIXED in `tests/multiplayer_v2.rs`; the underlying LIBRARY defect is
> open and belongs to `loft-libs-net`.** Two tests were written off as flaky across
> several full-suite runs (`v2_two_clients_with_spectator_routing`,
> `host_input_utf8_passthrough`). They are not the same problem, and the first is not a
> flake at all — it is a silent failure mode that a test harness could not see.

## The one-line finding

**`server::listen(port)` returns a `Server` that looks fine when the bind FAILED.**
`n_tcp_listen` returns `-1` on error, and `listen` wraps it verbatim:

```loft
pub fn listen(port: integer) -> Server { Server { handle: tcp_listen(port) } }
```

So a loft server whose port is already taken prints its own cheerful
`"v2 server listening on :36141"`, stays alive, and accepts nothing forever. Measured
directly — start two servers on one port and the loser reports:

```
loft_tcp_listen: cannot bind 0.0.0.0:36141: Address already in use (os error 98)
v2 server listening on :36141          <- the script's own line, after the failure
loser alive? YES
```

This is a **user-facing product bug, not a test bug**: any loft service that loses a port
comes up "healthy" and serves nothing. It lives in `loft-libs-net/server/` (also shipped
as `server-0.3.1`), so it is that repo's to fix; the honest signature is a nullable return
(`listen(...) -> Server?`) or a raise, so the caller can see it.

## Why it surfaced as a test flake

`tests/multiplayer_v2.rs` picks a port with the standard bind-`:0`-and-close idiom, which
has a TOCTOU window. Measured on this box (300 trials per burst):

| concurrent pickers | duplicate port handouts |
|---|---|
| 3 | 0 |
| 12 | 3 (~1 %) |
| 64 | 31 (~10 %) |

Nine test files use that idiom, so the effective burst under a full nextest run is far
above three. When two tests get the same port, one server loses the bind — and **the
readiness probe cannot tell**, because `wait_for_port` only asks *"is anything listening
here?"*, and the winner's server answers. The losing test then runs its clients against a
stranger's server. That produces exactly the two signatures seen:

- `v2_two_clients_with_spectator_routing` — A and B land on a server that is also pairing
  someone else's clients, so they are never paired with each other. Observed:
  `[A] Shutdown: own-game-complete-no-partner` with both clients otherwise healthy.
- `v2_single_client_completes_game` — reproduced locally, 1 in 20 runs of the three v2
  tests concurrently: the client printed `client v2 start` and hung for the full 60 s
  drain, having connected to a server running a different scenario.

## Theories tested and discarded

Both were plausible, cheap to check, and wrong — recorded so nobody re-runs them:

- **Machine resources.** No OOM kills; 50 GB free; `/tmp` is disk-backed with 1.4 TB free;
  `pid_max` is 4194304, so the pid-keyed native-binary path cannot realistically collide.
- **The readiness probe's connect-and-drop occupying a client slot.** The v2 server pairs
  "the first two connected clients", so a phantom seemed likely to steal slot 1. Tested
  head-on with a two-arm probe (with/without the phantom connect), 6 runs each: **6/6 both
  arms**. The server reaps it. Discarded.
- **Slow client startup under load.** The first reading of the evidence, and the one the
  fixture's own comment suggests. Falsified: at load average 47 on 24 cores, the
  two-client scenario observed its partner 10/10 with the stock 200 ms pause.

## The fix (harness half)

`spawn_server_on_free_port` replaces "pick a port, spawn, hope". It asks the server which
outcome it got — the native listener prints exactly one of `loft server listening on ...`
or `loft_tcp_listen: cannot bind ...` — and re-picks when it lost. Losing is an ordinary
event under a busy box and costs milliseconds to retry; what must never happen is
proceeding against someone else's server.

`server_detects_and_retries_a_stolen_port` is its positive control: it steals the first
pick on purpose and asserts both that a bare connect-probe still succeeds on the stolen
port (why `wait_for_port` alone could never catch this) and that the helper burns that
pick and returns a working server. Verified to FAIL when `bind_outcome` is neutered, so
it cannot go vacuous.

## `host_input_utf8_passthrough` — a separate, still-unexplained failure

Different shape: the `--native` leg produced empty stdout while `--interpret` was correct.
**Not reproduced** — 24 concurrent identical `--native` builds are clean, and the test
passes in isolation and under load. It shares nothing with the port race (it opens no
sockets).

What was fixable is that its harness threw the evidence away: `run()` used
`Stdio::null()` for stderr and returned `""`, so the assertion read
`left: "echo=[café] len=4\n", right: ""` — a symptom with the cause deleted. It now
returns a diagnostic carrying the exit status and stderr, verified to render a real
message. The next occurrence will say why instead of just that.
