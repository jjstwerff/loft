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
PE binary), see [NATIVE_DEBUG.md](../../plans/34-native-debug/README.md) — that path is
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

### Build order — small, safe, incremental

Each step is independently landable, has its own verify gate, and the server is a
**usable, shippable LSP the moment S3 lands** (diagnostics alone earns its keep on every
editor). The safety spine: build the transport *in isolation* first; grow the compiler
surface only as **additive accessors** returning structured data — never a rewrite of the
parse path; and test each step at the **protocol level** (a scripted harness, not a live
editor) so nothing silently regresses.

- **S0 — protocol harness (the instrument, first).** A driver that pipes
  `Content-Length`-framed JSON-RPC into `loft-lsp`'s stdin and asserts its stdout, so every
  step is CI-tested without an editor. *Gate:* it drives a handshake **and can fail** (feed a
  bad reply, see it caught) — a harness that can't fail proves nothing.
- **S1 — transport skeleton, no compiler.** `initialize` (advertise only
  `textDocumentSync=full`) → `initialized` → `shutdown`/`exit`. *Gate:* harness completes the
  handshake + clean exit; VS Code connects. Framing/encoding bugs live here — nail it before
  any feature touches it.
- **S2 — structured diagnostics — DONE, lighter than planned.** The plan assumed a ~60-site
  sweep to add positions; in fact loft ALREADY carries positioned, coded diagnostics (@I75;
  @PLN102 arc-E) — every `diagnostic!` site records `DiagEntry { level, line, col, message,
  code }`. So S2 was just the *recipe*: a fresh stdlib-loaded parser (`parse_dir("default")`)
  → `parse_source(buf)` → `diagnostics.entries()`. Gate: `tests/lsp_diagnostics.rs` (clean →
  none; syntax error correctly positioned; unknown symbol reported with its message). Two
  dogfood findings from building the consumer: **(a)** a parser **cannot be re-parsed** on a
  warm base — loft registers every definition *per source* (that is how files read each other on
  `use`), so a second `parse_source` re-registers and conflicts (*"Cannot redefine 'main'"*). So
  loft-lsp uses a **fresh parser per parse** — the correct model, not a stopgap (~80 ms, within
  budget). Incremental re-parse / warm-reuse is incompatible with the source model, not a perf
  step to chase. (An initial attempt to "reset" diagnostics for reuse was the wrong tree — the
  redefine conflict is upstream of diagnostics.)
  **(b) — FIXED.** Deferred/semantic errors (e.g. "Unknown function") used to report at the
  *resolution point* (the cursor's drifted resting place at the enclosing item's terminator),
  not the reference site — so they landed on the wrong line. Root cause: a call is
  type-checked in `parser::call()` *after* its arguments are fully parsed, by which time
  `self.lexer` has advanced past the statement; the seven `diagnostic!(self.lexer, …)` "Unknown
  function" sites read that drifted cursor. Fix: thread the identifier's own `name_pos`
  (already captured by `parse_var`) through `parse_call` → `dispatch_call` → `call()` and emit
  via `diagnostic_at!(self.lexer, name_pos, …)`. The caret now sits on the offending name
  itself (`nope` at line 2 col 3), not the paren or the `}`. Chokepoint fix — the whole
  "Unknown function" family (plain, `len`/`size` arity, method-hint, generic-type) moved at
  once. Regression: `tests/lsp_diagnostics.rs::unknown_symbol_is_reported_at_the_reference_site`
  pins `(2, 3)`; four existing `parse_errors`/`issues` position assertions were corrected from
  the old drifted columns to the name columns. Syntax errors were already exact.
- **S3 — publish diagnostics — DONE.** The document lifecycle (`didOpen` / `didChange` /
  `didClose`) drives a fresh-parse per edit and pushes `textDocument/publishDiagnostics`; the
  editor shows squiggles live and clears them when the buffer goes valid (empty list on a clean
  parse, and on close). The compiler coupling is a pure library accessor —
  `loft::lsp::diagnose(text, name, stdlib_dir)` (new `src/lsp.rs`): fresh stdlib-loaded parser →
  buffer → `Diagnostics`, encapsulating the parser-cannot-be-reparsed rule. The binary owns only
  the wire protocol + stdlib-path resolution (`resolve_stdlib_dir()`, exe-relative like the
  `loft` CLI, so it works from any editor CWD). Mapping: loft's 1-based `DiagEntry` → LSP 0-based
  `Diagnostic`; the single-point position is widened to underline the whole identifier
  (`token_len_at`, read from the buffer) so the squiggle covers the token, not a zero-width caret;
  `Level` → severity (Error/Fatal 1, Warning 2). Advertises `textDocumentSync {openClose, change:1}`.
  *Gate:* `tests/lsp_transport.rs::diagnostics_publish_on_open_then_clear_on_fix` drives the real
  spawned binary — asserts the pushed notification, uri, count, severity, message, and the exact
  range (`start (1,2) → end (1,6)` on `nope`), then edits clean and asserts the empty clear. The
  clean-buffer-clears assertion doubles as proof the stdlib resolved from the spawned binary.
  **Diagnostics-only is real value across every LSP editor — this is the shippable milestone.**
- **S4 — outline — DONE.** `textDocument/documentSymbol` lists the buffer's top-level defs
  (fn / method / struct / enum / typedef / constant / interface) in source order. The prereq #3
  accessor shipped as `loft::lsp::outline(text, name, stdlib_dir) -> Vec<Symbol>` (not a `Data`
  method — the fresh-parse recipe belongs with `diagnose`): enumerate `0..data.definitions()`,
  keep `def.source == MAIN_SOURCE` and non-`synthetic`, and read the kind + decoded name from the
  shared `api_surface::classify` (made `pub` — one home for the `n_`/`t_<LEN><Type>_`/`Op` name
  decoding) plus `def.position`. Finding: the parser records a def's `position` at the BODY start
  (past the name), so the LSP maps `range`/`selectionRange` to the name located on its declaration
  line (`name_range`, from the buffer) — the Outline entry jumps to the name, not the `{`. Kind →
  LSP `SymbolKind` (Struct 23, Function 12, Enum 10, …); advertises `documentSymbolProvider`.
  *Gate:* `tests/lsp_outline.rs` (3 — kinds+order, excludes stdlib/variants/synthetics, empty) on
  the lib accessor, and `tests/lsp_transport.rs::document_symbol_lists_the_outline` drives the real
  binary and asserts the reply list + the `selectionRange` landing on `Point` (line 0, chars 7..12).
- **S5 — hover — DONE (name-resolution scope).** `textDocument/hover` shows the resolved
  symbol's signature + its `///` doc as markdown. Shipped as `loft::lsp::symbol_at(text, name,
  stdlib_dir, line, col) -> Option<Hover>`. Two findings drove the design:
  - **Resolution.** Prereq #2 envisioned a per-file position index. S5 ships the lighter
    *name-based* resolver instead: the identifier under the cursor is looked up as `n_<word>`
    (free fn) then `<word>` (type/struct/enum/typedef/constant), each falling back to the stdlib
    — so hovering a call site (`area(2,3)`) resolves to the definition, and hovering `print`
    resolves into the stdlib. Not resolved: **methods** (`t_<LEN><Type>_…` need the receiver
    type) and **locals** (need scope) — those still want the position index, deferred to when
    S6/precise-resolution demands it.
  - **Docs (the "other way to get definition info").** loft keeps NO doc field — the lexer
    discards comments — but docs live as `///` lines in the `.loft` *source*, and every
    `Definition` carries `position.{file,line}` into real source (a stdlib symbol → e.g.
    `default/04_stacktrace.loft:41`). So hover reads the `///` block above the declaration from
    the definition's own source — the open buffer for local defs, the file on disk for
    stdlib/library ones (`stdlib_dir`-relative). This is the convention `gendoc` already relies
    on; the signature itself reuses `api_surface::signature_of` (made `pub`) for one type
    spelling. *Gates:* `tests/lsp_hover.rs` (5 — user-fn-at-call-site + doc, stdlib type + doc
    read cross-file from source, struct sig, off-word → None, unknown → None) and
    `tests/lsp_transport.rs::hover_shows_signature_and_doc` on the real binary.
- **S6 — go-to-definition — DONE.** `textDocument/definition` reuses `symbol_at` and emits an
  LSP `Location`: a LOCAL symbol jumps within the open document, a STDLIB / library symbol jumps
  into its source file (a `file://` uri under `default/`, canonicalized). To make the jump land on
  the NAME (matching the outline), `symbol_at` was extended to return the name-precise position +
  the resolved name — it already reads the def's source for docs, so it now also locates the name
  on the declaration line (`name_col_on_line`, shared with the `///` extraction as `read_def_source`
  / `doc_block_above`). Advertises `definitionProvider`. *Gate:*
  `tests/lsp_transport.rs::go_to_definition_jumps_to_local_and_stdlib_defs` on the real binary —
  local jump (same uri, name range), stdlib jump (`file://…/default/…`), blank → null. The S0
  positive control moved off `textDocument/definition` (now implemented) to `textDocument/completion`.

S0–S6 complete **LSP.1**. LSP.2 (below) and `loft-dap` (a DAP adapter over the **existing**
@PLN16 debugger engine — the engine is done, this is a protocol shim, not a new debugger)
layer on the same spine. Every step ships behind three checks: a unit test (compiler side),
a harness test (protocol side), and a real-editor smoke.

### Shipped ahead of LSP.2

Two LSP.2-surface items already landed on the S-spine, plus a cross-cutting perf win:

- **`textDocument/formatting` — DONE.** Wraps the same `loft fmt` formatter
  (`tools/fmt/whole.loft`) via `loft::lsp::Formatter` (compiled once, cached); returns one
  whole-document `TextEdit`, none when already tidy. Advertises `documentFormattingProvider`.
  Gate: `tests/lsp_transport.rs::formatting_returns_a_whole_document_edit_and_noops_when_tidy`.
- **Stdlib startup-cache warm-load — DONE (perf).** The parse accessors (`diagnose` / `outline` /
  `symbol_at`) re-parsed `default/` on every request — ~50 ms, the dominant per-edit cost. They
  now route through `load_stdlib()`, which `startup_cache::warm_load_stdlib`es the precompiled
  `Data` bundle (~4.8 ms, **~10×**) when present, else cold-parses + `save_stdlib_cache`. Verified
  the bundle round-trips `Definition.position`, so stdlib hover / go-to-def are unaffected. The
  binary defaults `LOFT_STDLIB_CACHE` on (honors an explicit override). This is the "integrate
  with the data we already keep" win — the CLI's stdlib cache, now shared by the LSP.
- **Agent/shell frontend — DONE.** The SAME `loft::lsp` accessors, exposed as one-shot `loft` CLI
  subcommands so scripts and coding agents reach the code intelligence without a live editor (a
  THIRD frontend beside the LSP server and the future browser IDE): `loft symbols <file>`
  (outline), `loft def <name> [file]` (signature + `///` doc + location by NAME — a free fn / type
  / const PLUS every `Type.name` method, so `def len` lists `text.len` / `vector.len` / …, the
  method resolution the cursor-based hover can't do), `loft hover <file> <ln> <col>`.
  Human-readable by default, `--json` for structure (mirrors `loft api`). New lib pieces:
  `loft::lsp::lookup()` + a shared `hover_of_def()`. Gate: `tests/lsp_cli.rs` drives the real
  binary. This is dogfood: it replaces the `grep default/*.loft` + read loop for "what's the
  signature of X".

### Tag integration — tracker knowledge in the IDE (loft dogfood)

Surface loft's own `@`-tracker system (issues / features / plans) inline, reading the
ALREADY-generated `index/tags.json` + `index/features.json` (`make index`; queried by
`scripts/idx`). Lights up when the workspace is the loft repo (or any tree using the tag
convention + `make index`); **inert elsewhere** (the index files are absent) — which fits the
audience exactly: agents/devs working ON loft. The index is generated so it can lag until the
next `make index` — advisory, like any index-backed feature. Steps:

- **T1 — hover on a tag — DONE.** A cursor on an `@`-tag token (`@P259` / `@PLN63` / `@F7` /
  `@I81` / `@GH247`) hovers its tracker info instead of a symbol. `@F`/`@I` pull the title +
  first-paragraph description from `features.json`; `@P`/`@GH`/`@PLN` show the first indexed
  reference's context; a deterministic issue URL per family (`@GH`→loft, `@PLN`→plans,
  `@F`/`@I`→features). Rendered as markdown with a clickable `[issue]` link.
- **T2 — document links — DONE.** `textDocument/documentLink` returns a link per tag that has an
  issue URL (`@GH`/`@PLN`/`@F`/`@I`), at the tag's range → ctrl-clickable. Advertises
  `documentLinkProvider`.

Prereq (DONE): `loft::lsp::TagIndex` reads `index/tags.json` (required) + `features.json`
(optional) from `<workspace_root>/index` (captured from `initialize`'s `rootUri`), parsed once +
cached; `tag_at` / `tags_in` detect tokens; `render_tag_markdown` formats. Also a **`loft tag
<@TAG>`** CLI (dogfood — the same lookup from the shell; walks up to `index/`). No new index — it
consumes the tracker index the repo already builds (`make index`). *Gates:* `tests/lsp_tags.rs`
(synthetic index — CI has no generated `tags.json`: feature/URL/summary lookup + token detection),
`tests/lsp_transport.rs::tag_hover_and_document_link` (temp workspace, real binary),
`tests/lsp_cli.rs`-adjacent CLI dogfood.

Remaining tag steps:

- **T3 — broken-tag diagnostics — DONE.** A buffer tag the scanner flagged as broken (a
  `@P`/`@PLAN` reference to no valid issue/plan) publishes a Warning at the tag, folded into the
  diagnostics push on open/change. **Consumes the index's `broken` array verbatim** rather than
  re-deriving validity: the scanner (`scan.loft::tag_is_broken`) reads PROBLEMS.md + the plan dirs
  + the freeze-banner `@P→#` map, so re-implementing it offline would duplicate it and risk false
  positives — the LSP inherits the scanner's exact verdict (`TagIndex::is_broken`). Zero false
  positives; the trade-off is that a freshly-typed broken tag not yet indexed only shows after the
  next `make index` (the index is the source of truth, consistent with `idx broken`). *Gates:*
  `tests/lsp_tags.rs::broken_tags_come_from_the_index_verdict` +
  `tests/lsp_transport.rs::broken_tag_publishes_a_warning` (a NON-broken tag draws no warning).
- **T4 — tag completion.** `@P…` / `@PLN…` → valid tags from the index (low priority).

Follow-up: the `TagIndex` is loaded once per session, so it can lag a mid-session `make index` —
add an mtime refresh when that friction shows up.

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
| `textDocument/codeAction` (`refactor.extract` — **extract function**) | Turn a SELECTION of statements into a new function: data-flow over the selection computes which locals are read-before-write (→ parameters) and which are written-then-used-after (→ return value(s); loft tuples cover multiple outputs), synthesize the fn (name + signature + body) and replace the selection with a call. **Protocol side is trivial** (a `CodeAction` carrying a `WorkspaceEdit`); the cost is the intra-function data-flow ENGINE, which reads the parser's per-fn variable/scope tables (`Definition.variables: Function`). loft specifics: honor the deps/ownership model on extracted params; handle `self` when inside a method; refuse across `#rust`/`#native` bodies. **L effort — not started; the most involved item.** |
| `textDocument/inlayHint` | Inline type annotations: parameter names at call sites, inferred types of `let`-style locals. |
| `textDocument/formatting` | **DONE** (shipped ahead of the rest of LSP.2) — runs the SAME formatter the `loft fmt` CLI uses (`tools/fmt/whole.loft`, compiled once via `loft::lsp::Formatter`), returns one whole-document `TextEdit` (none when already tidy). Gate: `tests/lsp_transport.rs::formatting_returns_a_whole_document_edit_and_noops_when_tidy`. (The old `loft --format` Rust formatter was removed; `loft fmt` is the entry point.) |

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

## Open work (routed in)

- **INSP.J — JSON output mode for `loft introspect`** (machine-readable
  bytecode / Rust / slot-table / type dumps).  Routed here from **@PLN12**
  (REPL + introspection) on its close: machine-readable introspection is an
  editor / IDE concern, and the LSP server is its natural consumer.  Small (S) —
  a JSON serializer over the existing `introspect::emit_all` structures; no new
  analysis.

---

## Cross-references

- [NATIVE_DEBUG.md](../../plans/34-native-debug/README.md) — GDB / LLDB integration for
  `--native`-compiled binaries; shares the source map with LSP.3.
- [WEB_IDE.md](../62-web-ide/README.md) — W2–W6 browser IDE; uses `loft-lsp`
  compiled to WASM as its language-intelligence layer.
- [DX.md](../../plans/36-developer-experience/README.md) — SH.1 / SH.2 / DX.1 / DX.3 / DX.4 — the 0.8.5
  developer-experience predecessors.
- [STACKTRACE.md](../../STACKTRACE.md) — TR1.3 `vector<StackFrame>` API
  that LSP.3 reuses for `stackTrace`.
- [Plan-14 viewer-LSP-bridge](../66-viewer-lsp-bridge/README.md)
  — the CLIENT side that consumes `loft-lsp` (this plan)
  alongside rust-analyzer + jdtls.  Plan-14 phase 03 lights
  up `.loft` files in the viewer once LSP.1 here ships.
