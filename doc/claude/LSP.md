<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Language Server + Debug Adapter — design

Loft's editor-integration story is built around two protocol-agnostic
servers that any modern IDE knows how to consume:

| Server | Protocol | Drives |
|---|---|---|
| `loft-lsp` | LSP (JSON-RPC over stdio) | code intelligence — diagnostics, completion, hover, go-to-def, rename, refactoring, semantic highlighting |
| `loft-dap` | DAP (JSON-RPC over stdio) | interpreter-mode debugging — launch, breakpoints, step, locals, watch |

One server unlocks **first-class support across every editor that
speaks the protocol**: VSCode, Eclipse (LSP4E + DSP4E), JetBrains
(LSP4IJ), Helix, Neovim, Sublime, the future browser IDE (W2), Emacs
(`eglot` + `dape`).  Per-IDE work shrinks to a thin Java / TS / Lua
plugin that just registers the `.loft` content type and points at the
binary.

For native-mode debugging (`loft --native` produces an ELF / Mach-O /
PE binary), see [NATIVE_DEBUG.md](NATIVE_DEBUG.md) — that path is
GDB / LLDB-driven and complementary; the source map is shared.

---

## Architecture

```
┌──────────────────┐       LSP / DAP        ┌──────────────────┐
│  IDE / editor    │  ◀──── JSON-RPC ────▶  │  loft-lsp        │
│  (VSCode, …)     │                        │  loft-dap        │
└──────────────────┘                        └────────┬─────────┘
                                                     │ in-proc
                                            ┌────────▼─────────┐
                                            │  loft (rlib)     │
                                            │   parser         │
                                            │   typedef        │
                                            │   scopes         │
                                            │   state          │
                                            └──────────────────┘
```

`loft-lsp` and `loft-dap` are new binaries in this repo (or eventually
in `loft-tools`) that link the existing `loft` rlib for parser /
typecheck / runtime access.  They translate JSON-RPC requests into
calls against `Parser`, `Data`, `State`.  No new compiler — the
existing one is the whole intelligence layer.

The thin per-IDE plugins (`loft-vscode`, `loft-eclipse`, `loft-jetbrains`)
contain only:
- `package.json` / `plugin.xml` declaring the `.loft` content type,
- a 50–200-line shim that spawns the right binary on activation.

---

## LSP.1 — MVP language server (0.8.6)

**Goal:** every LSP-capable editor gets diagnostics, document outline,
and hover on day one.  Smallest unit of work that delivers visible
value across all editors.

### Surface

| Method | Behaviour |
|---|---|
| `initialize` | Advertise capabilities: `textDocumentSync = 1` (full sync), `documentSymbolProvider = true`, `hoverProvider = true`, `diagnosticProvider`. |
| `textDocument/didOpen` | Parse the file, run typecheck, publish diagnostics. |
| `textDocument/didChange` | Re-parse from full text (incremental sync deferred to LSP.2). |
| `textDocument/didSave` | No-op (didChange already triggered a parse). |
| `textDocument/publishDiagnostics` | Emit `(range, severity, message, code)` for every error / warning the parser produces. |
| `textDocument/documentSymbol` | Walk `Data.definitions` for the file; emit a `DocumentSymbol[]` tree (struct → fields, fn → params).  Drives the IDE's Outline view. |
| `textDocument/hover` | At cursor `(line, col)`, look up the symbol and return its type, signature, and `///` doc-comment if present. |
| `shutdown` / `exit` | Cleanup; `loft-lsp` is per-workspace, not per-file. |

### Loft-side prerequisites

Three accessors that don't exist yet:

1. **`Parser::parse_text(text: &str, path: &Path) -> (Data, Vec<Diagnostic>)`**
   Today the parser writes errors to stderr and exits.  LSP needs them
   as a structured list.  The `Diagnostic` shape:
   ```rust
   pub struct Diagnostic {
       pub range: (Position, Position),  // (line, col) start + end
       pub severity: Severity,           // Error | Warning | Info | Hint
       pub message: String,
       pub code: Option<&'static str>,   // stable ID, e.g. "E0023" or "W-unused"
       pub related: Vec<(Range, String)>, // optional secondary spans
   }
   ```
   Source line/column are already tracked by the lexer (`Lexer::line` /
   `Lexer::col`); they need to be stamped into the `Diagnostic` rather
   than only into the eprintln string.
2. **`Data::symbol_at(file: &Path, pos: Position) -> Option<Symbol>`**
   Resolves a cursor position to a definition, function call, or
   variable reference.  Implemented as a lookup over per-file
   position indices built during parse.
3. **`Data::file_symbols(file: &Path) -> Vec<Symbol>`**
   All top-level definitions in the file, ordered by source position.
   Drives `documentSymbol`.

The first accessor is the heaviest — it requires plumbing diagnostic
positions through every `Self::error(...)` call site (~60 sites in the
parser).  None of the changes are deep; they're the kind of mechanical
sweep the comment-hygiene pass already established.

### Performance budget

LSP servers re-parse on every keystroke.  For a 1000-line file the
target is **sub-100 ms in release mode**.  Today a release-mode loft
binary parses and typechecks the entire stdlib + a 1k-line user file
in ~80 ms; on incremental edits to one file the budget should hold.
For 10k-line files, full re-parse becomes sluggish (~400 ms); LSP.2
introduces incremental sync to mitigate.

### Risks

- **Parser is two-pass and has hidden global state.**  Pass-1 registers
  every definition, pass-2 fills function bodies.  `Data::definitions`
  grows monotonically; re-parsing the same file from scratch should
  produce the same `Data` but only if pass-1 is idempotent across calls.
  Verify with a regression test that re-parses 10× and asserts identical
  symbol tables.
- **Single file vs. workspace.**  Loft programs are usually multi-file
  (`use foo;` references).  LSP.1 parses one file at a time; if the
  user has unsaved changes in `bar.loft` and types `use bar;` in
  `foo.loft`, the cross-file resolution is stale.  Acceptable for MVP;
  workspace-aware parsing is part of LSP.2.
- **Re-entrancy.**  An IDE may send `didChange` while the previous
  parse is still running.  Solution: serialise per-file via a per-URI
  `Mutex<ParserState>`, with the latest `didChange` superseding any
  in-flight one.

---

## LSP.2 — full editing surface (0.9.0)

**Goal:** parity with what JDT delivers for Java in Eclipse — every
operation a working developer expects from a "real language" IDE.

### Surface

| Method | Behaviour |
|---|---|
| `textDocument/completion` | Context-aware suggestions: members of `expr.`, params of a call, in-scope identifiers, keywords.  Sorted by relevance (in-scope first, then stdlib, then alphabetic). |
| `textDocument/definition` | Jump to the symbol's declaration.  Resolves through `use` chains. |
| `textDocument/references` | Find every read / write of the symbol across the workspace. |
| `textDocument/rename` | Rename in-place across the workspace; `prepareRename` first to validate the target is a renamable identifier (not a keyword / native fn). |
| `textDocument/semanticTokens` | Type-aware token classification: function vs. method vs. constant vs. field, mutable vs. const, locals vs. parameters.  Supersedes the SH.1 TextMate grammar's structural-only highlighting. |
| `textDocument/codeAction` | Quick-fixes: "add missing field", "rename to camelCase", "import `bar`".  Each diagnostic with a known fix produces an action. |
| `textDocument/inlayHint` | Inline type annotations: parameter names at call sites, inferred types of `let`-style locals. |
| `textDocument/formatting` | Run `loft --format` on the buffer (T2-0 prerequisite). |

### Loft-side prerequisites

1. **Workspace symbol index.**  A `Data` per file is fine for LSP.1;
   LSP.2 needs a `Workspace` aggregate with cross-file resolution and
   incremental update on `didChange`.  Naturally a HashMap keyed by
   `(file, def_nr)` plus reverse indices keyed by name and by
   `Symbol → Vec<Reference>`.
2. **Completion engine.**  At cursor `(file, line, col)` resolve the
   syntactic context (after `expr.`, inside fn-call args, top-level)
   and return a ranked candidate list.  ~MH effort — the first
   completion that's *helpful* not *noisy* takes work.
3. **Fix-it catalogue.**  Most diagnostics already know the fix
   ("add `&` here", "type was `text`, expected `integer`").  Surface
   each as a `WorkspaceEdit` the IDE can apply.

### Incremental parsing

LSP.2 introduces partial re-parse: on `didChange` with small ranges,
re-parse only the affected function body.  Loft's parser is
top-down recursive-descent without global state inside `parse_function`,
so re-parsing one function in isolation is feasible.  ~M effort.
Skip until LSP.1 measurements show real users hitting the 10k-line wall.

### Risks

- **Rename across `#native` boundaries.**  A user can't rename a function
  whose Rust body lives in `#rust "..."` annotations without breaking
  the binding.  The fix-it should refuse with a clear message.
- **Rename that touches imported libraries.**  The workspace includes
  vendored / installed packages; renaming a stdlib function would be
  catastrophic.  Restrict rename to definitions whose source file is
  inside the project root.
- **Performance.**  Workspace-wide find-references on a 50k-line
  project must complete in under 1 s; otherwise developers stop trusting
  the feature.  Pre-build a `Symbol → Vec<Reference>` index during
  initial parse.

---

## LSP.3 — `loft-dap` debug adapter (0.9.0)

**Goal:** interactive interpreter-mode debugging in any DAP-aware
editor.  Set a breakpoint in `.loft` source, run, hit the breakpoint,
inspect locals, step.

### Surface

| Request | Behaviour |
|---|---|
| `initialize` | Capabilities: `supportsConfigurationDoneRequest = true`, `supportsConditionalBreakpoints = true`, `supportsHitConditionalBreakpoints = true`, `supportsExceptionInfoRequest = true`. |
| `launch` | Spawn a child loft interpreter process with `LOFT_DAP_PORT=$port` env var; the interpreter connects back and registers as the debuggee. |
| `setBreakpoints` | Translate `.loft` `(file, line)` to a bytecode position; install a breakpoint flag on that opcode. |
| `configurationDone` | Resume the debuggee from its initial pause. |
| `threads` | Return the single thread (or one per parallel worker). |
| `stackTrace` | Return the `vector<StackFrame>` from TR1.3. |
| `scopes` | Per frame: `Locals`, `Arguments`, `Globals`. |
| `variables` | Walk the named slots in the requested scope; format using `Data` types. |
| `next` / `stepIn` / `stepOut` | Single-step at the source-line granularity. |
| `continue` / `pause` | Run / interrupt. |
| `evaluate` | Evaluate a small loft expression in the current frame's scope (LSP.3 v1 only supports identifier / field-access / call). |
| `disconnect` | Tear down the debuggee. |

### Loft-side prerequisites

1. **In-process pause API.**  Today the interpreter runs to completion
   (or panics).  Add a global `PauseFlag` checked at every opcode
   dispatch in `src/state/mod.rs::execute`.  Set it from a separate
   thread that owns the DAP socket.
2. **Source-line breakpoint resolution.**  The codegen already records
   `(opcode → loft_line)` mappings for crash reports.  Expose this as
   a `Data::breakpoint_for(file, line) -> Vec<(d_nr, code_pos)>`
   accessor.  Set the pause flag at the matching opcodes.
3. **Variable formatter.**  Loft's `ShowDb::write` already produces
   user-readable output for any `DbRef`.  Reuse it for the `variables`
   reply, with a depth limit to avoid descending into cyclic
   `vector<Reference>` graphs.
4. **Conditional-breakpoint expression evaluator.**  Reuse the parser
   on a single expression, lift it onto a synthetic frame with the
   current locals as inputs.  ~M effort; v1 can refuse complex
   expressions.

### Multi-worker support

`par(...)` and `parallel { ... }` spawn workers that have their own
`Stores` instances.  DAP `threads` returns one entry per active
worker; `stackTrace` operates per worker.  Pausing one worker pauses
all (synchronous-stop semantics) so the user sees a consistent picture.

### Risks

- **Pause-flag overhead.**  Checking a flag at every opcode dispatch
  costs ~1 ns × 10^9 ops = 1 s of overhead in a tight loop.  Acceptable
  during a debug session; needs a way to disable cleanly when no
  debugger is attached.  Solution: feature-gate the check behind a
  `#[cfg(feature = "dap")]` and ship two interpreter binaries (the
  default release build has DAP support disabled).
- **Breakpoint timing.**  A breakpoint set "before" the function is
  parsed (e.g. on a library file the program hasn't reached yet) needs
  to be applied retroactively.  Solution: keep a `pending_breakpoints`
  list, replay it at every parse.
- **Reverse stepping.**  DAP supports `stepBack` / `reverseContinue`
  via `supportsStepBack`.  Loft can't replay a tree-walking interpreter
  cheaply; v1 does not advertise this capability.
- **Debugger-induced state changes.**  `evaluate` could mutate state
  (e.g. `evaluate("x = 5")`).  v1 evaluates in read-only mode; mutations
  require explicit user opt-in.

---

## Eclipse plugin (1.0.0 — IDE.ECLIPSE)

A ~200-line Java OSGi bundle that registers `.loft` with the LSP4E
generic editor and the DSP4E launcher.  No Loft-specific Java code
beyond the bindings.

### Files

```
loft-eclipse/
├── plugin.xml          (manifest: content type, launch config, …)
├── META-INF/MANIFEST.MF
├── src/
│   └── org/loft/eclipse/
│       ├── LoftLanguageServer.java   (extends LSP4E ProcessStreamConnectionProvider)
│       ├── LoftDebugAdapter.java     (extends DSP4E DebugAdapterDescriptorFactory)
│       └── LoftActivator.java        (OSGi bundle activator)
└── icons/
    └── loft.png        (the file-type icon)
```

### `plugin.xml` skeleton

```xml
<plugin>
  <extension point="org.eclipse.core.contenttype.contentTypes">
    <content-type id="org.loft.contentType"
                  name="Loft Source"
                  base-type="org.eclipse.core.runtime.text"
                  file-extensions="loft" />
  </extension>

  <extension point="org.eclipse.lsp4e.languageServer">
    <server id="org.loft.languageServer"
            class="org.loft.eclipse.LoftLanguageServer"
            label="Loft Language Server" />
    <contentTypeMapping contentType="org.loft.contentType"
                        id="org.loft.languageServer" />
  </extension>

  <extension point="org.eclipse.debug.core.launchConfigurationTypes">
    <launchConfigurationType id="org.loft.debug"
                             name="Loft Program"
                             delegate="org.eclipse.lsp4e.debug.launcher.DSPLaunchDelegate" />
  </extension>

  <extension point="org.eclipse.lsp4e.debug.debugAdapterDescriptorFactories">
    <factory class="org.loft.eclipse.LoftDebugAdapter"
             launchConfigurationType="org.loft.debug" />
  </extension>
</plugin>
```

### `LoftLanguageServer.java` skeleton

```java
public class LoftLanguageServer extends ProcessStreamConnectionProvider {
  public LoftLanguageServer() {
    var loft = findLoftLsp(); // PATH or bundled binary
    setCommands(List.of(loft.toString()));
    setWorkingDirectory(System.getProperty("user.dir"));
  }
  private Path findLoftLsp() {
    // 1. $LOFT_LSP env var if set
    // 2. ~/.loft/bin/loft-lsp if installed via `loft install`
    // 3. PATH lookup for `loft-lsp`
    // Falls back with an actionable error message.
  }
}
```

### Marketplace listing

Eclipse Marketplace requires a hosted P2 update site.  Use the standard
Tycho build (`mvn tycho`) inside `loft-eclipse/`.  CI builds the update
site and uploads to GitHub Pages alongside the rest of the docs; the
Marketplace listing points at that URL.  ~1 day of one-off setup, then
zero ongoing cost — every release rebuilds the update site as part of
`make gallery`.

### Optional polish

| Feature | Effort | Status for IDE.ECLIPSE v1 |
|---|---|---|
| Project wizard ("New → Loft Project") | S | Skipped; users use Generic Project |
| Run-config UI (vs. plain DSP4E launch) | S | Skipped; default DSP4E launcher is fine |
| Custom debug perspective layout | S | Skipped; default Debug perspective works |
| Outline view icon set | XS | Skipped; LSP `documentSymbol` maps to default icons |
| Keybindings (F3 go-to-def, etc.) | XS | LSP4E provides these out of the box |

---

## JetBrains plugin (1.0.0 — IDE.JETBRAINS)

LSP4IJ ([JetBrains/lsp4ij](https://github.com/redhat-developer/lsp4ij))
is the JetBrains-side analogue of LSP4E.  Plugin shape mirrors the
Eclipse one: `plugin.xml`, a `LanguageServerFactory`, and pointer at
`loft-lsp`.

The JetBrains marketplace handles all platforms — IntelliJ Community
/ Ultimate, RustRover, PyCharm, GoLand, WebStorm, etc.  One plugin
listing covers them all.

`loft-dap` is wired through LSP4IJ's `DAPRunConfiguration`.  Same
shape as the Eclipse path.

---

## Neovim (1.0.0 — IDE.NEOVIM)

No plugin.  Just a snippet that the user drops into their
`init.lua`:

```lua
-- ~/.config/nvim/lua/loft.lua
require('lspconfig').configs.loft = {
  default_config = {
    cmd = { 'loft-lsp' },
    filetypes = { 'loft' },
    root_dir = require('lspconfig.util').root_pattern('loft.toml', '.git'),
  },
}
require('lspconfig').loft.setup{}

-- nvim-dap configuration for native + interpreter debug
local dap = require('dap')
dap.adapters.loft = {
  type = 'executable',
  command = 'loft-dap',
}
dap.configurations.loft = {
  {
    type = 'loft',
    request = 'launch',
    name = 'Run current file',
    program = '${file}',
  },
}
```

Loft ships this in `doc/` as `nvim-loft.lua`.  No Vimscript.

---

## Sequencing across milestones

| Milestone | LSP work | DAP work | IDE plugins |
|---|---|---|---|
| 0.8.5 | (SH.1, SH.2 — TextMate grammar + VSCode bare-bones extension) | — | — |
| 0.8.6 | LSP.1 — diagnostics + outline + hover | — | (none — LSP.1 lights up VSCode + Eclipse + Neovim immediately via the existing LSP4E / nvim-lspconfig integrations) |
| 0.9.0 | LSP.2 — completion + def + refs + rename | LSP.3 — DAP MVP | — |
| 1.0.0 | (polish only) | (polish only) | IDE.ECLIPSE / IDE.JETBRAINS / IDE.NEOVIM dedicated marketplace plugins |
| 1.1+ | (ongoing — formatter, inlay hints, semantic refactors) | (call hierarchy, conditional breakpoints v2) | (Sublime, Helix, Emacs `eglot` snippets) |

---

## Cross-references

- [NATIVE_DEBUG.md](NATIVE_DEBUG.md) — GDB / LLDB integration for
  `--native`-compiled binaries; shares the source map with LSP.3.
- [WEB_IDE.md](WEB_IDE.md) — W2–W6 browser IDE; uses `loft-lsp`
  compiled to WASM as its language-intelligence layer.
- [DX.md](DX.md) — SH.1 / SH.2 / DX.1 / DX.3 / DX.4 — the 0.8.5
  developer-experience predecessors.
- [STACKTRACE.md](STACKTRACE.md) — TR1.3 `vector<StackFrame>` API
  that LSP.3 reuses for `stackTrace`.
