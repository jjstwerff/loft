<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Loft in Eclipse

Full loft support in Eclipse — **code intelligence** (diagnostics, hover, completion,
go-to-definition, find-references, rename, semantic highlighting, extract-function) and
**interpreter-mode debugging** (breakpoints, stepping, locals, watch, data breakpoints,
reverse execution) — through three community plugins pointed at loft's own binaries:

| Eclipse plugin | Talks to | loft binary |
|---|---|---|
| **LSP4E** (Language Server Protocol for Eclipse) | LSP | `loft-lsp` |
| **DSP4E** (`org.eclipse.lsp4e.debug`) | Debug Adapter Protocol | `loft-dap` |
| **TM4E** (TextMate for Eclipse) | TextMate grammar | `syntaxes/loft.tmLanguage.json` |

Nothing loft-specific is reimplemented for Eclipse: the same `loft-lsp` / `loft-dap` that
serve VS Code and Neovim serve Eclipse too. This directory only declares the `.loft` content
type and wires the binaries in.

## 0. Prerequisites (both paths)

1. **Build the binaries** and put them on your `PATH` (or note their absolute paths):
   ```sh
   cargo build --release --bin loft-lsp --bin loft-dap
   export PATH="$PWD/target/release:$PATH"   # or copy the two binaries somewhere on PATH
   ```
2. **Eclipse 2023-09 (4.29) or newer** with the three plugins installed — *Help → Eclipse
   Marketplace…* and search for, or install from the update sites:
   - **LSP4E** and **TM4E** — https://download.eclipse.org/lsp4e/releases/latest/
   - **DSP4E** (the "Debug Adapter" support) ships in the same LSP4E feature (bundle
     `org.eclipse.lsp4e.debug`).

## Path A — try it now, no build (recommended first)

Eclipse's LSP4E/TM4E/DSP4E can be wired entirely from the **Preferences** UI, so you can try
loft in minutes without building the plugin.

### A1 · Content type + syntax highlighting (TM4E)
1. *Preferences → General → Content Types* → select **Text**, **Add…** a child (or use
   *Text → Loft*), and add the file association `*.loft`.
2. *Preferences → TextMate → Grammars* → **Add…** → pick this repo's
   `editors/eclipse/org.loft.eclipse.ide/syntaxes/loft.tmLanguage.json` (scope `source.loft`),
   then bind it to the `*.loft` content type.
3. *(optional)* *Preferences → TextMate → Language Configurations* → **Add…** →
   `editors/eclipse/org.loft.eclipse.ide/language-configuration.json` for bracket matching and
   comment toggling.

Open a `.loft` file — it should now be syntax-highlighted.

### A2 · Code intelligence (LSP4E user-defined server)
*Preferences → Language Servers* → **Add…** a user-defined server:
- **Associated content-type:** `Loft Source File` (the one from A1)
- **Command:** `loft-lsp`  *(or the absolute path to the binary)*

Reopen a `.loft` file — diagnostics, hover, completion, outline, rename, and semantic
highlighting come live.

### A3 · Debugging (DSP4E launch)
*Run → Debug Configurations… → Debug Adapter Launcher* → **New**:
- **Launch a Debug server / adapter** → **Launch command:** `loft-dap`
- Debug mode: **Launch** with a JSON `launch` request:
  ```json
  { "program": "${workspace_loc:/YourProject/path/to/file.loft}", "stopOnEntry": false }
  ```
Set breakpoints in the `.loft` editor gutter and press **Debug**. Reverse stepping
(`stepBack` / `reverseContinue`) and data breakpoints are offered because `loft-dap`
advertises them.

## Path B — the packaged plugin (`org.loft.eclipse.ide`)

A PDE plugin that bundles A1–A2 (content type + grammar + language server) so users don't
configure anything by hand. Build/run it from an Eclipse **with PDE + the target platform**
(LSP4E, TM4E installed as above):

1. *File → Import… → General → Existing Projects into Workspace* → select
   `editors/eclipse/org.loft.eclipse.ide`.
2. Try it live: right-click the project → *Run As → Eclipse Application* — a second Eclipse
   opens with loft support active; open any `.loft` file.
3. Ship it: *Export… → Plug-in Development → Deployable plug-ins and fragments* to produce a
   drop-in jar (or *Export → Deployable features* for an update site).

The plugin resolves the binary from the `loft.lsp` system property, then the `LOFT_LSP`
environment variable, else `loft-lsp` on the `PATH` (see `LoftLanguageServer.java`). The
debug side still uses the DSP4E launch from **A3** (a bundled launch shortcut is a follow-up).

> **Note:** the Java/OSGi bundle here is a convention-correct starting point but has **not**
> been compiled against a specific LSP4E/TM4E release in this repo's CI (there is no Eclipse
> build environment here). If an extension point or API differs on your LSP4E version, fall
> back to **Path A** — it drives the exact same binaries and is the verified route. Please
> report any adjustment needed so the plugin can be pinned to a known-good target platform.

## Status

`@PLN63` LSP.1–LSP.3 shipped; this is the Eclipse client wiring toward the **IDE.ECLIPSE**
milestone (a signed marketplace plugin is the 1.0.0 follow-up). See
[../../doc/claude/lib_plans/63-lsp/README.md](../../doc/claude/lib_plans/63-lsp/README.md).

## Troubleshooting

- **No highlighting / no diagnostics:** confirm `loft-lsp` runs from a terminal
  (`loft-lsp` should wait on stdin). If it's not on `PATH`, set the absolute command in the
  Language Servers preference (A2) or `LOFT_LSP` for the plugin (B).
- **Debug won't start:** confirm `loft-dap` runs (`loft-dap` waits on stdin), and that the
  `program` path in the launch config points at a real `.loft` file.
- **`loft` vs `loft-lsp`/`loft-dap`:** these are separate binaries from the same crate —
  build all with `cargo build --release --bin loft-lsp --bin loft-dap`.
