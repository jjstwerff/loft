<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 06 — Closeout + colleague-onboarding doc

**Status:** Open (depends on phases 00-05)

## Goal

Close plan-14 by documenting the multi-language code-intelligence
experience for the people the user named the recruiting target:
formidable programmers evaluating loft as a tool platform.

Phase 06 ships the documentation that turns "we have it" into
"any colleague can use it from a cold install in under 10
minutes."  Then move plan-14 to `lib_plans/finished/`.

## What ships

### `doc/claude/DEBUG.md` § Multi-language code intelligence in `make view`

A new top-level section covering:

- **What works**: hover, jump-to-def, references for Rust /
  loft / Java files.
- **First-launch checklist**:
  - Install rust-analyzer (`rustup component add rust-analyzer`).
  - Install jdtls + JDK 17+ (per-OS install paths documented).
  - loft-lsp ships in the loft binary (no separate install).
- **Verification flow**: `make view`, open `src/main.rs`,
  hover over `fn main`, expect tooltip within 200 ms (warm
  pool) or 30 s (cold start).  Repeat per language.
- **Troubleshooting**:
  - "Hover does nothing": check bridge is running
    (`pgrep loft-lsp-bridge`); check log
    (`tail -f /tmp/loft-lsp-bridge-<pid>.log`).
  - "rust-analyzer not found": install steps + env-var
    override.
  - "Cold start takes minutes": rust-analyzer indexing first
    time; subsequent restarts hit warm pool.
  - "Java doesn't work": JDK detection + jdtls install path.
- **Logs panel**: where it lives in the UI, what it shows,
  how to filter.
- **Keybind reference**: full table.
- **Per-OS notes**: Linux / macOS / Windows (if Windows
  shipped per phase 00 risk table).

### Public-facing onboarding doc

A separate `doc/CODE_INTELLIGENCE.md` (note: at the root
`doc/` level, not `doc/claude/`) that's the FIRST READ for
a colleague being shown the viewer:

- 3-paragraph overview of what the viewer + LSP gives them.
- Screenshots: hover popup, refs sidebar, diagnostics panel,
  keybind help overlay (one screenshot each — focus on
  visual quality).
- 5-minute quickstart: clone repo, install rust-analyzer,
  `make view`, port-forward, open browser.
- "Why we built this" — links to plan-14 README's drivers
  section.
- "How to extend" — adding a new language is N hours; pointer
  to phases 03 / 04 as templates.

This doc is what the user shares with colleagues.  It's the
recruiting story made concrete.

### CHANGELOG entries

`CHANGELOG.md` (user-facing):

```markdown
## 0.X.0 (date)

### Code intelligence in the viewer

`make view` now ships full LSP-based code intelligence for
Rust, loft, and Java files: hover popups with type signatures
+ docs, jump-to-definition, find references, inline error +
warning diagnostics.  Powered by a new `loft-lsp-bridge`
sidecar binary that consumes rust-analyzer, loft-lsp, and
jdtls.

Install rust-analyzer / JDK + jdtls; the loft side ships
in the binary.  See `doc/CODE_INTELLIGENCE.md` for the full
walkthrough.

Closes plan-14.
```

`CHANGELOG_TECHNICAL.md` (contributor-facing):

```markdown
## 0.X.0

### plan-14 (lib) — viewer-LSP-bridge SHIPPED

- 7 phases, ~2 quarters of focused work.
- New binary: `tools/loft-lsp-bridge/` (Rust; ~3500 LoC).
- New library: `lib/lsp_bridge_client/` (loft client wrapper).
- Viewer: ~2000 lines of vanilla JS overlay (no framework
  yet — phase 07+ promotes to CodeMirror).
- Architecture: 3 IPC layers (browser↔viewer WebSocket,
  viewer↔bridge length-prefixed JSON over Unix socket,
  bridge↔servers stdio JSON-RPC).
- Bridge intelligence: warm pool, multiplex, document cache,
  debounce, crash recovery, structured tracing.
- Languages: Rust (rust-analyzer), loft (loft-lsp from
  plan-09), Java (jdtls).
- Quality bar: hover P95 ≤ 50 ms warm; cold start ≤ 2 s
  warm-pool hit; crash recovery transparent.
- 12 new test crates (`tests/lsp_bridge_*.rs`); fixtures in
  `tests/fixtures/lsp/`.
- Documentation: DEBUG.md § Multi-language code intelligence;
  doc/CODE_INTELLIGENCE.md (colleague onboarding); per-phase
  closeout retrospectives.
```

### Move to `lib_plans/finished/14-viewer-lsp-bridge/`

Standard closeout: `git mv` the directory; update the
`lib_plans/README.md` table to reflect "Finished".

### Cross-doc updates

- `CLAUDE.md` § Key commands: `make view` row mentions
  "+ multi-language LSP (Rust/loft/Java)" — keeps the
  Key Commands as the discovery surface.
- `ROADMAP.md`: row added under § F — Foundation /
  Tooling, marked closed with a pointer to
  `lib_plans/finished/14-viewer-lsp-bridge/`.
- `lib_plans/finished/09-lsp/` (if plan-09 has shipped by
  this point): cross-link to plan-14 as "the client side
  of this server".

### CI gate additions

- `tests/doc_hygiene.rs` extends to verify
  `doc/CODE_INTELLIGENCE.md` exists and links to plan-14.
- `tests/lsp_bridge_smoke.rs` — minimal end-to-end: spawn
  bridge, attach a real rust-analyzer, request hover on a
  fixture, assert response shape.  Runs in CI on Linux
  (rust-analyzer pre-installed in CI image).  Lower
  priority on macOS / Windows lanes — gated by feature
  flag if CI install of rust-analyzer is too heavy.

## Acceptance

1. `doc/claude/DEBUG.md` § Multi-language code intelligence
   exists and is link-target-valid.
2. `doc/CODE_INTELLIGENCE.md` exists with the 4 screenshots
   + quickstart + extension pointer.
3. CHANGELOG entries land in both user-facing + technical
   variants.
4. Plan-14 directory moved to `lib_plans/finished/14-…/`.
5. `lib_plans/README.md` table reflects "Finished".
6. ROADMAP.md row added under § F — Foundation / Tooling.
7. CLAUDE.md § Key commands updated.
8. CI gate `tests/lsp_bridge_smoke.rs` passes on Linux lane.
9. No broken cross-references (verified via plan-37 phase 03
   broken-link auditor, which by then catches both `@PLAN-id`
   and markdown file links per plan-37 phase 09).

## Risks

| Risk | Mitigation |
|---|---|
| Documentation lands but the experience drifts | Acceptance includes a real walkthrough on a clean VM by someone other than the implementer; if it takes > 10 min from cold install to first hover, the doc + UX are wrong and the closeout slips. |
| Screenshots get stale | `doc/CODE_INTELLIGENCE.md` links to a `gallery/lsp/` page that gets regenerated by `make gallery` (existing infrastructure); screenshots refreshed each release. |
| Colleague onboarding finds bugs we missed | Plan-14 closeout retrospective explicitly invites colleague feedback; bugs file as new P-issues, NOT as plan-14 reopens.  Plan-14 closes; bugs get their own scope. |
| ROADMAP entry implies future commitment | Closed-plan rows are read-only history; ROADMAP § F lists shipped work.  No new commitments inferred. |

## Cross-references

- All phase docs (00-05).
- [Plan-14 README — Acceptance](README.md#acceptance--full-plan)
  — the criteria phase 06 verifies.
- [Plan-14 README — Quality bar](README.md#quality-bar-the-colleague-evaluator-framing)
  — the five metrics that map to the closeout walkthrough.
- [`plans/_LIFECYCLE.md`](../../../plans/_LIFECYCLE.md) — the
  active → finished move convention.
