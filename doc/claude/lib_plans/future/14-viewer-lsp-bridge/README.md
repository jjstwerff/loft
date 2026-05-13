<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# VIEWER_LSP_BRIDGE — multi-language LSP integration for loft-view

**Status:** Future — opened 2026-05-13.

A sidecar Rust binary (`loft-lsp-bridge`) plus loft-side wiring
that turns `make view` from a doc-and-code BROWSER into a real
multi-language CODE INTELLIGENCE surface — hover, jump-to-def,
references, eventually completion + refactoring — for `.rs`,
`.loft`, and `.java` files in any project the viewer is pointed
at.

The bridge is the LSP CLIENT side; it consumes existing LSP
SERVERS (rust-analyzer, jdtls) and the loft-lsp server from
[`lib_plans/future/09-lsp/`](../09-lsp/README.md) once that ships.

## Drivers

Three layered drivers, each independently load-bearing:

1. **The viewer needs to be more than a doc renderer.**  As of
   plan-35 phase 02 + plan-37 phase 04a, `loft-view` serves
   directory tree, code with line numbers, and tracker-tag
   references.  Reviewing branches in the chat console is now
   tolerable — but landing in any code file is still
   "syntax-highlighted-text + breadcrumbs".  No hover, no
   jump-to-def, no references.  For real review, that's not
   enough.

2. **Loft needs to demonstrate itself as a tool platform.**
   The user's framing (2026-05-13): *"This is exactly the type
   of tooling I like and a big reason to start with loft."*
   The viewer + LSP integration is the recruiting story.
   Colleagues evaluating loft will judge by how the tools
   built ON loft feel — not just how the language reads.
   "Done right" sets a quality bar: latency, completeness,
   crash recovery, multi-language even-handedness.

3. **First-class loft tooling means treating loft files
   alongside rust + java, not as a special case.**  Plan-09
   (`lib_plans/future/09-lsp/`) builds the loft-lsp SERVER —
   the language-intelligence backend for `.loft` files.  This
   plan builds the CLIENT — the harness that consumes
   loft-lsp the same way it consumes rust-analyzer or jdtls.
   Together they give `.loft` files genuine first-class
   treatment: same hover UI, same definition-jump UX, same
   refs-sidebar shape as rust-analyzer delivers for `.rs`.

## Architecture

Three IPC layers, only one of which is constrained by an
external spec:

```
[Browser]  ◀─ A: WebSocket ─▶  [loft-view]  ◀─ B: Unix socket ─▶  [loft-lsp-bridge]
                               (loft script)                       (Rust binary)
                                                                          │
                                                          C: stdio JSON-RPC
                                                                          │
                                                ┌─────────────────────────┼─────────────────────┐
                                                ▼                         ▼                     ▼
                                          rust-analyzer              loft-lsp                jdtls
                                          (existing)              (plan-09 LSP.1)         (existing)
```

| Layer | Constraint | Choice | Why |
|---|---|---|---|
| **A** browser ↔ viewer | Browser sandbox: WebSocket or HTTP | WebSocket (already in lib/server) | Reuses existing primitive; bidirectional; binary frames available for big payloads |
| **B** viewer ↔ bridge | None — we own both ends | Unix domain socket + length-prefixed JSON via `tokio-util::Framed` | Async + cancellable; cross-platform via `interprocess` crate; clean shutdown semantics; no stdio fragility |
| **C** bridge ↔ LSP servers | LSP base protocol: stdio + `Content-Length:` framing | Standard library use of `lsp-server` crate | Battle-tested by rust-analyzer team; corner cases (server crash, framing edge cases, JSON-RPC ID tracking) already solved |

Each layer is independently testable.

## Why a Rust sidecar (not pure loft, not fused-into-viewer)

Five reasons, each load-bearing:

1. **Loft has no subprocess primitives today.**  Verified
   2026-05-13: no `n_spawn`/`n_process`/`n_exec` in `src/`,
   no `process()`/`pipe()` in `default/*.loft`.  Pure-loft
   path requires first building those primitives — substantial
   stdlib expansion that's not ABOUT LSP.  Sidecar avoids that
   blocking dependency.

2. **Battle-tested protocol code exists in Rust.**  The
   `lsp-server` crate (rust-analyzer team) handles every LSP
   protocol corner case: shutdown races, capability
   negotiation, dynamic registration, framing edge cases,
   JSON-RPC ID tracking.  Reimplementing in loft burns months
   and ships bugs that mature libs already fixed.

3. **Reusable across loft tooling.**  The same
   `loft-lsp-bridge` becomes the LSP client used by the
   future browser IDE (`lib_plans/future/07-web-ide/`),
   terminal editors, anywhere loft-tooling needs LSP.  One
   binary, many consumers — same model as `loft-index`
   (plan-37 phase 07).

4. **Independent release cycle.**  LSP-server quirks (and
   rust-analyzer / jdtls / loft-lsp all have them) get fixed
   without churning the viewer's loft script.  Sidecar
   binaries can be installed via `loft install` (plan-37 phase
   08 deploys to `~/bin/`) and updated independently.

5. **Matches existing precedent.**  Plan-37 already splits
   the indexer into `loft-index` (daemon, plan-37 phase 07) +
   `loft-idx` (CLI).  This codebase organises around small
   binaries with focused responsibilities; the bridge is the
   third such binary.

The fused-binary alternative (Rust LSP bridge baked INTO
loft-view) is rejected: the viewer needs to be replaceable as
the loft script evolves; the bridge needs to outlive viewer
restarts (server warm-pool cache); and the boundary helps
keep concerns honest.

## What the bridge DOES (the differentiator)

A naive bridge is a JSON proxy: take a request from the
viewer, forward to the LSP server, echo back the response.
That works but doesn't justify a Rust sidecar — could be
done in 200 lines of loft once subprocess primitives exist.

The interesting bridge is one that USES its position to add
value the LSP servers cannot:

| Capability | Why the bridge is the right place | Without it |
|---|---|---|
| **Server warm pool** | rust-analyzer takes ~30 s to index a workspace.  Bridge keeps it alive across viewer restarts. | Every `Ctrl+C make view` cycle repays the indexing cost. |
| **Multi-client multiplex** | Two browser tabs share one rust-analyzer instance via the bridge's per-server fan-out. | Each tab spawns its own rust-analyzer → 2× memory + indexing time. |
| **Per-document state cache** | Bridge caches offsets, semantic-token revisions, last-known-good ASTs.  Cancelled requests don't lose work. | Re-fetch from server on every cancellation → wasted CPU + latency. |
| **Crash recovery** | Server dies, bridge respawns + replays open-document state from cache. | Browser sees broken state; user has to manually reopen files. |
| **Backpressure / debounce** | Viewer fires `didChange` on every keystroke; bridge collapses bursts before forwarding. | Server flooded → falls behind → diagnostics lag the cursor by seconds. |
| **Multi-server routing** | One viewer-side request routes to the right server by file extension; bridge owns the dispatch table. | Viewer learns each server's quirks → bloats the loft script. |
| **Structured tracing** | `tracing` crate emits per-request spans; logs answer "why is this hover slow". | "It's broken" with no diagnostic surface. |
| **Schema translation** | Bridge speaks one normalised JSON-RPC dialect to the viewer; LSP-server-specific quirks hidden behind it. | Every viewer-side caller learns LSP's full surface. |

These are what separate "wired up" from the "done right" bar
the user's colleagues will judge by.

## Browser-side editor — staged

The viewer surface in the browser starts small and grows.
Each stage is independently shippable + reviewable:

| Stage | What ships | Editor framework | Effort |
|---|---|---|---|
| **R1 — read-only nav** | Click any identifier → jump-to-def page; hover any symbol → tooltip with type + docs; sidebar lists references for the file. | None — augment the existing `<pre><code>` render with `<a href>` overlays. | S (one phase) |
| **R2 — inline diagnostics + hover popup** | Squiggle-underlines for errors/warnings; richer hover popup with multiple sections; "go to references" popup panel. | Still no editor framework — JS overlay on top of the existing code render. | S |
| **E1 — edit single file** | Edit-in-place; save through to disk via the viewer; LSP `didChange` events flow; diagnostics live-update. | CodeMirror 6 (lightweight, ~150 KB, modular) | M |
| **E2 — completion + signature help** | Autocomplete dropdown; inlay hints for params + inferred types; format-on-save. | CodeMirror 6 with autocomplete extension | M |
| **E3 — refactoring + multi-file** | Rename across files; quick-fixes (import, add field, rename to camelCase); workspace-wide refs panel. | CodeMirror 6 with workspace-edit support | M-L |

**Recommendation: ship R1 + R2 first** (one quarter combined),
then evaluate whether the user/colleague feedback justifies the
editor framework jump for E1.  Read-only nav covers 80% of the
"review someone's branch" use case the viewer was originally
built for.

Monaco vs CodeMirror choice deferred to E1 phase doc.  CodeMirror
6 is the current recommendation: smaller bundle, modular
plug-in surface, no IE legacy weight, well-supported LSP plug-in
([`codemirror-languageserver`](https://github.com/FurqanSoftware/codemirror-languageserver)).

## Quality bar (the colleague-evaluator framing)

Five concrete metrics that map to "done right":

1. **Cold start to first diagnostic**: ≤ 2 s from `make view`
   open to first error squiggle on a 1k-line `.rs` file.  Warm
   start (server pool hit): ≤ 200 ms.
2. **Hover latency**: ≤ 50 ms from cursor-stop to tooltip
   visible (P95).  Anything more feels broken.
3. **Multi-language even-handedness**: same shape of UI
   (hover popup, definition jump, refs sidebar) across `.rs`,
   `.loft`, `.java`.  Not "rust gets the rich UI; loft gets a
   subset."
4. **Crash recovery**: rust-analyzer killed externally → bridge
   detects, respawns, replays open-document state → browser
   never sees a broken cursor; user notices a < 1 s pause.
5. **Server log surfacing**: a "View LSP logs" link in the
   viewer footer opens the bridge's `tracing` log for the
   current session.  When something breaks, the colleague can
   see the actual JSON-RPC traffic, not just "broken".

These metrics drive acceptance for each phase.

## Phases

| # | Phase | Effort | What ships | Status |
|---|---|---|---|---|
| 0 | [Scaffold the bridge binary](00-scaffold.md) | M | `loft-lsp-bridge` Rust binary; Unix-socket protocol with length-prefixed JSON; echo-only (no LSP servers spawned yet); `lib/lsp_bridge_client/` loft library that wraps the socket. | Open |
| 1 | [rust-analyzer end-to-end](01-rust-analyzer.md) | L | Bridge spawns rust-analyzer, forwards `initialize`/`hover`/`definition`/`references`; viewer renders hover popups + jump-to-def + refs sidebar for `.rs` files in the loft repo. | Open |
| 2 | [Bridge intelligence](02-bridge-intelligence.md) | L | Server warm pool; multi-client multiplex; per-document state cache; debounce/backpressure; crash recovery; structured tracing.  Each capability acceptance-tested individually. | Open |
| 3 | [loft-lsp integration](03-loft-lsp.md) | M | Once `lib_plans/future/09-lsp/` LSP.1 ships, bridge spawns `loft-lsp` for `.loft` files.  Same hover / def / refs UX as rust-analyzer.  First-class loft treatment. | Open (depends on plan-09 LSP.1) |
| 4 | [Java via jdtls](04-jdtls.md) | M | Bridge spawns jdtls (Eclipse JDT-LS) for `.java` files.  `LOFT_JDTLS_HOME` env var or auto-discovery.  Same UI shape across all three languages. | Open |
| 5 | [Browser editor R1 + R2](05-browser-editor.md) | M | Read-only nav + diagnostics layer in the browser.  No editor framework yet.  Completes the "review dashboard" framing.  E1 (CodeMirror inline edit) is a stretch follow-up. | Open |
| 6 | [Closeout + colleague-onboarding doc](06-closeout.md) | S | DEBUG.md § "Multi-language code intelligence in `make view`"; install instructions for rust-analyzer/jdtls auto-discovery; CHANGELOG; move plan to finished/. | Open |

Total estimated effort: **2 quarters of focused work** (each
phase ~2-4 weeks).  Phases 0-2 are the architectural backbone;
phases 3-5 each light up one capability slice.

## Acceptance — full plan

- `make view` on the loft repo opens a browser; clicking any
  identifier in `src/parser/expressions.rs` jumps to its
  definition; hovering shows the type signature + doc comment.
- Same UX works on `.loft` files (via loft-lsp from plan-09)
  and `.java` files (via jdtls).
- Bridge survives `Ctrl+C` + restart cycle of `make view`
  without re-indexing rust-analyzer (warm pool hit).
- Killing rust-analyzer externally (SIGKILL) triggers bridge
  respawn + automatic state replay; browser shows < 1 s
  pause but no broken state.
- Two browser tabs on the same workspace share one
  rust-analyzer process (multiplex acceptance test).
- Cold-start ≤ 2 s; hover latency ≤ 50 ms P95; warm-start ≤
  200 ms.
- A "View LSP logs" footer link surfaces the bridge's
  `tracing` log for the current session.
- DEBUG.md gains a § "Multi-language code intelligence" with
  install + troubleshooting + colleague-friendly screenshots.
- All 7 phases close → plan moves to `lib_plans/finished/14-…`.

## Risks

| Risk | Mitigation |
|---|---|
| LSP servers have version skew (rust-analyzer ships breaking changes) | Bridge advertises minimum supported server version per language; surfaces a clear error when version is too old.  Pin `lsp-types` to a specific spec version per phase. |
| Java auto-discovery is fragile (many JDK distributions) | Phase 04 ships explicit `LOFT_JDTLS_HOME` env var + install doc; auto-discovery is a stretch.  No magic. |
| Browser-side editor scope creeps | R1 + R2 (read-only) is the explicit phase 05 ship.  E1 / E2 / E3 are filed as stretches; defer until R2 has been used in anger. |
| Server warm pool consumes idle memory | Bridge has a TTL: idle server killed after 30 min.  Wake-on-request is fast (server start + indexing happens once per workspace, then re-indexing is incremental). |
| Multi-client multiplex creates ID-collision bugs | Bridge owns its own request ID space and rewrites IDs in/out per client.  Per-client request map + cancellation tracked per client.  Pinned by `tests/lsp_bridge_multiplex.rs`. |
| `tokio` async + Unix sockets fragile on Windows | Use `interprocess` crate which abstracts Unix sockets / Windows named pipes.  Phase 00 includes a Windows CI lane. |
| Bridge binary must be installed for the viewer to work | `make view` checks for the bridge binary at startup; if missing, prints a clear "run `cargo install --path tools/loft-lsp-bridge`" message and falls back to the existing read-only viewer (graceful degradation, not a hard fail). |
| LSP-server logs leak sensitive content (file paths, project structure) | Footer "View LSP logs" link is opt-in; default is logs go to a per-session file under `/tmp/loft-lsp-bridge-<pid>.log`, not to the browser. |

## Cross-references

- [`lib_plans/future/09-lsp/README.md`](../09-lsp/README.md)
  — the loft-lsp SERVER side (LSP.1 / LSP.2 / LSP.3 / DAP);
  this plan is the CLIENT side and consumes it for `.loft`
  files in phase 03.
- [`plans/35-branch-review-viewer/README.md`](../../plans/35-branch-review-viewer/README.md)
  — the viewer this plan extends.  The viewer's layout
  (sidebar, breadcrumbs, code rendering) is the host for the
  new LSP-driven UI elements.
- [`plans/37-tracker-index/README.md`](../../plans/37-tracker-index/README.md)
  — the tracker-tag indexer.  Its `/tag/<bare>` route already
  shows the same kind of cross-reference UX the LSP "find
  references" sidebar will.
- [`lib_plans/future/07-web-ide/README.md`](../07-web-ide/README.md)
  — future browser IDE; reuses `loft-lsp-bridge` as its
  language-intelligence layer.
- [`plans/future/27-developer-experience/README.md`](../../plans/future/27-developer-experience/README.md)
  — DX umbrella; the viewer + LSP is one of the largest DX
  wins on the roadmap.

## Why this is a separate plan from plan-09

Plan-09 (`lib_plans/future/09-lsp/`) builds the loft-lsp
SERVER — the language-intelligence backend that knows about
`.loft` files specifically.  Plan-14 (this one) builds the
LSP CLIENT — a multi-language harness that consumes loft-lsp
along with rust-analyzer and jdtls.

The two are independently valuable:

- Plan-09's loft-lsp serves any LSP-capable editor (VSCode,
  Eclipse, Helix, Neovim) without needing the viewer.
- Plan-14's bridge serves the viewer (and future browser
  IDE) without needing loft-lsp — rust-analyzer + jdtls
  alone justify it.

They COMPOSE in plan-14 phase 03: once both are live, `.loft`
files in the viewer get the same first-class treatment as
`.rs` files do via rust-analyzer.

Splitting also keeps each plan focused.  Plan-09 is about
"loft language intelligence"; plan-14 is about "multi-language
client tooling around the viewer."  Different design surface,
different test surface, different phase shape.
