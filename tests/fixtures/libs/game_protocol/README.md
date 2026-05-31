<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# game_protocol — game-networking framing for loft

Per-tick state-update + event framing protocol used by the
TTT (tic-tac-toe) multiplayer demo and the audience-generative-art
demo.  Pure-loft (no native code).  Depends on `web` + `server`
for the underlying transport.

## Install

```sh
loft install game_protocol   # pulls web + server as transitive deps
```

## Surface

Packet header encoding, sequence numbers, ack/retransmit logic.
See `src/game_protocol.loft` for the type + function reference.

## Provenance

Extracted from `lib/game_protocol/` 2026-05-24.
