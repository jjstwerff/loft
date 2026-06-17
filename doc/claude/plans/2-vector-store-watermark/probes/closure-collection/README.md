<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# closure ↔ collection limitation probes

A closure-feature limitation surfaced (but **not** caused) by the rc-removal probing.
**Not rc-related** — these crash / reject with rc *on*.  Split out of the rc-removal
corpus so it doesn't confound "what does rc removal need".

The root is the same on both sides: **the closure-record layout (a 16-byte fn-ref slot —
4B `d_nr` + 12B closure DbRef — holding the captured payload as a flat attribute list)
does not model a collection's content-type indirection** (`P257`).

| probe | direction | rc-on | status |
|---|---|---|---|
| `01_collection_into_closure` | a `vector`/`hash`/… is **captured into** a closure body | clean **compile error** | **intended** — `P257` rejection (`objects.rs:229`), with a bind-the-element workaround |
| `02_closure_into_collection` | a **capturing closure is stored into** a vector element | **runtime panic** (`store.rs:1385`, read-only/locked store) | **bug** — the inverse case is NOT rejected; it crashes |

## Findings

- `01` (collection → closure) is a deliberate, documented restriction with a clean
  diagnostic + workaround.  No action.
- `02` (closure → collection) is the **un-handled inverse** of the same limitation.  IR
  confirms a *capturing* closure element is built into the vector via a 4-byte
  `OpSetInt4` write for a 20-byte fn-ref, and building the closure record writes to a
  read-only store → panic.  **Non-capturing** closures and **named fn-refs** in a vector
  work (no closure record), so it is specifically the *captured-cell* record.
- **Fix options:** (a) clean-reject storing a capturing closure into a collection,
  consistent with `P257`'s `01` rejection (bounded — needs detecting a capturing-closure
  value at the element site); or (b) model the closure record in a collection element
  (the real feature — deep, the same layout work `P257` defers).

This belongs with the closure-record-layout / `P257` family, not plan-57's store-lifetime
work — re-home if a closure plan opens.
