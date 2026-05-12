<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 4 — Hosting

## Status

Open.  Operational, not code (mostly).  Effort: **XS**.

## Goal

A public URL audience members can reach from their phones during
the talk.  Reliable enough that a 30-person room can all connect
without manual intervention.

## Target environment

**Linux server.**  Resolved 2026-05-10.  The server runs on a
Linux host — either the presenter's laptop (Linux), a small VPS,
or a Raspberry Pi-class box on the venue network.  Mac / Windows
hosting is **not a target** for the demo; loft itself runs
cross-platform but the demo's hosting plane is Linux-only to
simplify deployment + monitoring + backup-restart procedures.

## Deployment options

| Option | Pros | Cons |
|---|---|---|
| **Presenter's Linux laptop on hotspot** | Total control; no upstream provider; no public DNS needed | Requires the laptop to stay on stage stable during the talk; phone hotspot capacity may cap the audience size |
| **Small VPS (DigitalOcean / Hetzner / Fly.io)** | Always-on; public DNS; no venue WiFi dependency for the *server* (audience phones still need internet) | Costs a few dollars; exposes server to the open internet (need to harden) |
| **Raspberry Pi on venue network** | Cheap; bring-your-own-network independence | Venue WiFi politics; need IP-discovery story for audience phones |

## Sub-tasks

| # | Task | Effort |
|---|---|---|
| 4.1 | Pick deployment option (laptop / VPS / Pi) | XS |
| 4.2 | Harden the server for public exposure (basic rate-limit; reject malformed frames; log connection counts) | S |
| 4.3 | Set up the public URL — short / memorable.  QR-code generator from the URL | XS |
| 4.4 | Smoke test: 5+ phones from outside the local network connect successfully | XS |
| 4.5 | Backup plan — second host ready if primary dies during talk | XS |

## Open design questions

- **Which deployment option?**  Defer to phase 5 (rehearsal)
  decision — pick whichever holds up best under a real-room test.

## See also

- [`README.md`](README.md) — parent plan
- [`05-rehearsal-and-backup.md`](05-rehearsal-and-backup.md) —
  end-to-end smoke test
