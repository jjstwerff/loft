# Loft Web IDE — Design Plan

## Overview

A fully serverless, single-origin HTML application that lets users write, run,
and explore Loft programs inside a browser.  No install, no account, no server.
The Loft interpreter runs as a WebAssembly module compiled from the existing
Rust source; the IDE shell is plain ES-module JavaScript with no build step
required to open and use `index.html`.

### Goals

| Goal | Notes |
|---|---|
| Zero-server | Runs from `file://`, any static host, or a CDN |
| Full interpreter | Same Rust codebase, compiled to WASM via `wasm-pack` |
| Lightweight IDE | CodeMirror 6 editor, problems panel, console, outline |
| Navigate symbols | Go-to-definition, find-usages (Ctrl+click / sidebar) |
| Multi-project | All projects stored in IndexedDB; project switcher |
| Docs & examples | Bundled doc content and example projects inline |
| Export | One-click ZIP with a structure ready for `loft` locally |
| Offline | PWA service worker; works without a network after first load |

---

## Architecture

```
┌─────────────────────────────── Browser ─────────────────────────────┐
│                                                                      │
│  index.html                                                          │
│  ├── app.js           — top-level orchestrator                       │
│  ├── editor.js        — CodeMirror 6 instance                        │
│  ├── loft-language.js — Lezer highlight grammar for Loft             │
│  ├── wasm-bridge.js   — WASM loader + typed JS wrapper               │
│  ├── projects.js      — IndexedDB CRUD                               │
│  ├── symbols.js       — go-to-def, find-usages                       │
│  ├── docs.js          — documentation panel                          │
│  ├── export.js        — ZIP export / import (JSZip)                  │
│  └── examples.js      — bundled example projects                     │
│                                                                      │
│  pkg/loft_wasm.js   <──────────┐                                     │
│  pkg/loft_wasm_bg.wasm         │                                     │
│                                │ wasm-bindgen                        │
└────────────────────────────────┼─────────────────────────────────────┘
                                 │
        ┌────────────── Rust (wasm feature) ──────────────┐
        │  src/wasm.rs            — public WASM API        │
        │  src/fill.rs            — op_print → thread_local│
        │  src/diagnostics.rs     — add structured field   │
        │  src/lexer.rs           — populate structured    │
        │  src/parser/mod.rs      — virtual FS hook        │
        └─────────────────────────────────────────────────┘
```

---

## File & Directory Structure

```
ide/
├── index.html                 Entry point; loads WASM + mounts UI
├── style.css                  Layout, theme tokens (light + dark)
├── manifest.json              PWA manifest
├── sw.js                      Service worker (M6)
├── src/
│   ├── app.js                 Orchestrator: wires panels, keyboard shortcuts
│   ├── editor.js              CodeMirror 6 setup, error decorations, key maps
│   ├── loft-language.js       Lezer StreamLanguage for Loft syntax
│   ├── wasm-bridge.js         WASM init, compile_and_run(), get_symbols()
│   ├── projects.js            IndexedDB: open/save/list/delete projects
│   ├── symbols.js             Symbol index, go-to-def, find-usages
│   ├── docs.js                Docs panel: search + render bundled content
│   ├── export.js              ZIP export (JSZip) + ZIP import
│   └── examples.js            Bundled example project registry
├── tests/
│   ├── runner.html            In-browser test runner (no build tools)
│   ├── wasm-bridge.test.js    WASM compile + run integration tests
│   ├── projects.test.js       IndexedDB CRUD (mock IDB)
│   ├── export.test.js         ZIP structure validation
│   ├── loft-language.test.js  Token classification
│   └── symbols.test.js        Symbol extraction correctness
├── assets/
│   ├── examples/              Bundled .loft files (from tests/docs/)
│   └── docs-bundle.json       Pre-processed doc content (generated)
└── pkg/                       wasm-pack output (gitignored)
    ├── loft_wasm.js
    └── loft_wasm_bg.wasm
```

**Build commands** (only needed to regenerate WASM or the doc bundle):

```sh
# Build WASM module
wasm-pack build --target web --out-dir ide/pkg -- --features wasm

# Re-bundle docs (run after cargo run --bin gendoc)
node ide/scripts/bundle-docs.js

# Everything
./ide/build.sh
```

Opening `ide/index.html` directly in a browser is sufficient for development
after the WASM has been built once.

---

## Rust Changes Required

### 1 — `wasm` Cargo feature

```toml
# Cargo.toml additions
[features]
wasm = ["dep:wasm-bindgen", "dep:serde", "dep:serde-wasm-bindgen", "dep:js-sys"]

[dependencies]
wasm-bindgen        = { version = "0.2",  optional = true }
serde               = { version = "1",    optional = true, features = ["derive"] }
serde-wasm-bindgen  = { version = "0.6",  optional = true }
js-sys              = { version = "0.3",  optional = true }

[lib]
crate-type = ["cdylib", "rlib"]
```

### 2 — Structured diagnostics (`src/diagnostics.rs`)

Add a `structured` field alongside the existing `lines` vec so the IDE can
display file/line/col without parsing text strings.

```rust
pub struct DiagEntry {
    pub level:   Level,
    pub file:    String,
    pub line:    u32,
    pub col:     u32,
    pub message: String,
}

pub struct Diagnostics {
    lines:      Vec<String>,           // existing — unchanged
    structured: Vec<DiagEntry>,        // NEW
    level:      Level,
}
```

`Diagnostics::add()` gains a variant `add_at(level, file, line, col, msg)` that
pushes to both vecs.  The `diagnostic!` macro stays unchanged; `Lexer::diagnostic()`
calls `add_at` with `self.position.{file,line,pos}`.

### 3 — Thread-local output buffer (`src/fill.rs`)

`op_print` (line 1791) currently calls `print!("{}", v_v1.str())`.
Add a `#[cfg(feature = "wasm")]` branch:

```rust
// fill.rs — op_print
#[cfg(feature = "wasm")]
{ crate::wasm::output_push(v_v1.str()); }
#[cfg(not(feature = "wasm"))]
{ print!("{}", v_v1.str()); }
```

The thread-local buffer in `src/wasm.rs`:

```rust
thread_local! {
    static OUTPUT: RefCell<String> = RefCell::new(String::new());
}
pub fn output_push(s: &str) { OUTPUT.with(|o| o.borrow_mut().push_str(s)); }
pub fn output_take() -> String { OUTPUT.with(|o| mem::take(&mut *o.borrow_mut())) }
```

### 4 — Virtual filesystem (`src/parser/mod.rs`)

`lib_path()` resolves `use <name>` via the real filesystem.
For WASM, a thread-local `HashMap<String, String>` (filename → content) is
checked first:

```rust
#[cfg(feature = "wasm")]
thread_local! {
    static VIRT_FS: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

// At the top of lib_path():
#[cfg(feature = "wasm")]
if let Some(content) = VIRT_FS.with(|fs| fs.borrow().get(name).cloned()) {
    return Some(VirtFile { name: name.to_string(), content });
}
```

The WASM API populates `VIRT_FS` before parsing starts.

### 5 — `src/wasm.rs` (new file)

Public surface:

```rust
#[wasm_bindgen]
pub fn compile_and_run(files_js: JsValue) -> JsValue
// Input:  [{name: string, content: string}]
// Output: {output: string, diagnostics: DiagEntry[], success: bool}

#[wasm_bindgen]
pub fn get_symbols(files_js: JsValue) -> JsValue
// Input:  [{name: string, content: string}]
// Output: [{name, kind, file, line, col, usages: [{file,line,col}]}]
```

`compile_and_run` flow:
1. Populate `VIRT_FS` from `files_js`.
2. `Parser::new()` → `parse_dir(default/)` → `parse_str(main content)`.
3. `scopes::check` → `interpreter::byte_code` → `state.execute("main")`.
4. Collect `output_take()` + `parser.diagnostics.structured()`.
5. Clear `VIRT_FS`.
6. Serialize result with `serde_wasm_bindgen::to_value`.

---

## JavaScript API Contract

### `wasm-bridge.js`

```js
// Returns Promise that resolves once WASM is ready.
export async function initWasm(wasmUrl)

// Runs a project.  Returns RunResult.
// files: [{name: string, content: string}]
export function compileAndRun(files)
// → {output: string, diagnostics: Diagnostic[], success: boolean}

// Returns symbol table for navigation.
export function getSymbols(files)
// → Symbol[]
```

Types:
```ts
Diagnostic {
  level:   'error' | 'warning' | 'debug',
  file:    string,
  line:    number,   // 1-based
  col:     number,   // 1-based
  message: string
}

Symbol {
  name:    string,
  kind:    'function' | 'struct' | 'enum' | 'variable',
  file:    string,
  line:    number,
  col:     number,
  usages:  {file: string, line: number, col: number}[]
}
```

### `projects.js`

```js
// Returns all project summaries (id, name, modified).
export async function listProjects()

// Returns full project including all files.
export async function loadProject(id)

// Saves (creates or updates).  Returns saved project.
export async function saveProject(project)

// Deletes project and all its files.
export async function deleteProject(id)
```

Project schema:
```js
{
  id:       string,     // uuid
  name:     string,
  modified: number,     // Date.now()
  files: [
    { name: string, content: string }
  ]
}
```

### `export.js`

```js
// Returns a Blob (application/zip).
export async function exportZip(project)

// Returns a Project object (not saved — caller decides).
export async function importZip(blob)
```

Export ZIP layout:
```
<project-name>/
  src/
    main.loft
    <other .loft files>
  lib/
    <library .loft files if any>
  README.md           (auto-generated with project name + run instructions)
  run.sh              (#!/bin/sh\nloft src/main.loft "$@")
  run.bat             (@echo off\nloft src\main.loft %*)
```

---

## Development Roadmap

### M1 — WASM Foundation

**Goal**: Compile the interpreter to WASM; verify compile+run roundtrip from JS.

**Rust changes**
- Add `wasm` feature to `Cargo.toml`
- `src/diagnostics.rs`: add `DiagEntry` struct + `structured` field + `add_at()`
- `src/lexer.rs`: call `add_at()` with position in `Lexer::diagnostic()`
- `src/fill.rs`: `op_print` → thread-local buffer under `#[cfg(feature="wasm")]`
- `src/wasm.rs`: `compile_and_run()` only (no symbols yet)
- `src/parser/mod.rs`: virtual FS thread-local + populate at start of `compile_and_run`

**JS deliverable**: `ide/src/wasm-bridge.js` with `initWasm()` + `compileAndRun()`

**JS tests** (`tests/wasm-bridge.test.js`):
```js
test('hello world', async () => {
  const r = compileAndRun([{name:'main.loft', content:'fn main(){println("hi");}'}]);
  assert(r.output === 'hi\n');
  assert(r.success === true);
  assert(r.diagnostics.length === 0);
});

test('compile error returns diagnostic', async () => {
  const r = compileAndRun([{name:'main.loft', content:'fn main(){ bad! }'}]);
  assert(r.success === false);
  assert(r.diagnostics.some(d => d.level === 'error'));
  assert(r.diagnostics[0].line >= 1);
  assert(r.diagnostics[0].col  >= 1);
});

test('multi-file: use resolved from virtual FS', async () => {
  const files = [
    {name:'main.loft', content:'use helper;\nfn main(){println(greet());}'},
    {name:'helper.loft', content:'fn greet() -> text {"hello"}'}
  ];
  const r = compileAndRun(files);
  assert(r.output === 'hello\n');
});

test('runtime output captured', async () => {
  const code = 'fn main(){ for i in 1..4 { print("{i} "); } }';
  const r = compileAndRun([{name:'main.loft', content: code}]);
  assert(r.output === '1 2 3 ');
});
```

**Milestone check**: `wasm-pack build` succeeds; all 4 tests pass.

---

### M2 — Minimal Editor Shell

**Goal**: A working `index.html` a user can open, write code in, and run.

**Layout** (no frameworks — plain CSS grid):
```
┌──────────────────────────────────┐
│  toolbar: [project▾] [run ▶]    │
├──────────────────┬───────────────┤
│                  │  Console      │
│  Editor          ├───────────────┤
│  (CodeMirror 6)  │  Problems     │
│                  │               │
└──────────────────┴───────────────┘
```

**JS deliverables**:
- `ide/src/loft-language.js` — `StreamLanguage.define()` tokenizer for Loft:
  - Keywords: `fn if else for in return const pub use struct enum boolean true false`
  - Types: `integer long float boolean text vector`
  - Operators, string literals (with `{...}` interpolation spans), line comments `//`
  - Block comments `/* */`
- `ide/src/editor.js` — CodeMirror 6 instance:
  - Language, line numbers, bracket matching, auto-close brackets, dark/light theme
  - `setDiagnostics(diags)` — decorates error/warning lines with gutter icons + underlines
- `ide/index.html` — loads WASM, mounts editor, wires Run button

**JS tests** (`tests/loft-language.test.js`):
```js
test('keyword tokens', () => {
  const tokens = tokenize('fn main() {}');
  assert(tokens[0] === {text:'fn',    type:'keyword'});
  assert(tokens[1] === {text:'main',  type:'function-def'});
});

test('string with interpolation', () => {
  const tokens = tokenize('"value={x}"');
  assert(tokens.some(t => t.type === 'string'));
  assert(tokens.some(t => t.type === 'interpolation'));
});

test('line comment', () => {
  const tokens = tokenize('// comment\nfn');
  assert(tokens[0].type === 'comment');
  assert(tokens[1].type === 'keyword');
});

test('type names highlighted', () => {
  const tokens = tokenize('a: integer');
  assert(tokens.some(t => t.text === 'integer' && t.type === 'type'));
});

test('number literal', () => {
  const tokens = tokenize('42');
  assert(tokens[0].type === 'number');
});
```

**Milestone check**: open `index.html`, type a loft program, click Run, see output in Console; errors show in Problems with line numbers.

---

### M3 — Symbol Index & Navigation

**Goal**: Go-to-definition and find-usages work via Ctrl+click.

**Rust changes** (`src/wasm.rs`):
- Implement `get_symbols(files_js: JsValue) -> JsValue`
- Walk `parser.data.def_names` + variable tables; collect:
  - Function definitions: name, kind=`"function"`, file, line, col
  - Struct/enum type defs: kind=`"struct"` / `"enum"`
  - Top-level variables in `fn main` scope: kind=`"variable"`
  - For each definition: scan IR for references → `usages` list

**JS deliverables**:
- `ide/src/symbols.js`:
  - `buildIndex(symbols)` → `Map<name, Symbol>`
  - `findAtPosition(index, file, line, col)` → `Symbol | null`
  - `formatUsageList(symbol)` → HTML string for usages panel
- `editor.js` extension:
  - `Ctrl+click` → `findAtPosition` → `editor.setCursor(def.line, def.col)`
  - Hover tooltip showing kind + file of symbol
- Outline panel (collapsible sidebar): lists all `function` and `struct`/`enum` symbols; clicking navigates

**JS tests** (`tests/symbols.test.js`):
```js
test('finds function definition', () => {
  const syms = [{name:'greet', kind:'function', file:'main.loft', line:1, col:4, usages:[]}];
  const idx  = buildIndex(syms);
  const found = findAtPosition(idx, 'main.loft', 1, 5);
  assert(found.name === 'greet');
});

test('find-usages returns all references', () => {
  // Simulate a symbol with 3 usages
  const sym = {name:'add', kind:'function', file:'main.loft', line:1, col:4,
               usages:[{file:'main.loft',line:10,col:8},{file:'main.loft',line:20,col:4}]};
  const html = formatUsageList(sym);
  assert(html.includes('line 10'));
  assert(html.includes('line 20'));
});

test('no match returns null', () => {
  const idx = buildIndex([]);
  assert(findAtPosition(idx, 'main.loft', 5, 5) === null);
});
```

**Milestone check**: Ctrl+click a function call → editor jumps to its definition. Right-click → "Find usages" shows list; clicking an entry navigates.

---

### M4 — Multi-File Projects

**Goal**: Full project management; each project has named files; stored in IndexedDB.

**JS deliverables**:
- `ide/src/projects.js` — IndexedDB wrapper:
  - Store `projects` (schema in API Contract above)
  - `listProjects()`, `loadProject(id)`, `saveProject(project)`, `deleteProject(id)`
  - Auto-save on every edit (debounced 2 s)
- UI additions to `app.js`:
  - Project switcher dropdown (toolbar)
  - "New project" dialog (prompts name + optional template)
  - File tree panel (left sidebar): list files, add/rename/delete
  - Tab bar for open files
  - `use` keyword auto-complete for filenames in current project
- All `compileAndRun` calls pass the full project `files[]`

**JS tests** (`tests/projects.test.js`):
```js
// Uses fake-indexeddb (npm dev-dep, or inline mock)
test('save and load roundtrip', async () => {
  const p = {id:'abc', name:'Test', modified: 0,
             files:[{name:'main.loft', content:'fn main(){}'}]};
  await saveProject(p);
  const loaded = await loadProject('abc');
  assert(loaded.files[0].content === 'fn main(){}');
});

test('list returns all projects', async () => {
  await saveProject({id:'p1', name:'A', modified:1, files:[]});
  await saveProject({id:'p2', name:'B', modified:2, files:[]});
  const list = await listProjects();
  assert(list.length === 2);
  assert(list.some(p => p.name === 'A'));
});

test('delete removes project', async () => {
  await saveProject({id:'del', name:'D', modified:0, files:[]});
  await deleteProject('del');
  const list = await listProjects();
  assert(!list.some(p => p.id === 'del'));
});

test('auto-save updates modified timestamp', async () => {
  const p = {id:'ts', name:'T', modified:0, files:[]};
  await saveProject(p);
  p.files.push({name:'a.loft', content:''});
  const saved = await saveProject(p);
  assert(saved.modified > 0);
});
```

**Milestone check**: create two projects, switch between them; each remembers its own files and state.

---

### M5 — Documentation & Examples Browser

**Goal**: Users can read the language docs and open any example as a project
without leaving the IDE.

**Build-time step** (`ide/scripts/bundle-docs.js`):
- Parse `doc/06-function.html`, `doc/07-vector.html`, etc.
- Extract section headings + prose + code blocks into `assets/docs-bundle.json`:
  ```json
  [
    { "id": "06-function",
      "title": "User defined functions",
      "sections": [
        { "heading": "Declaring Functions",
          "prose": "...",
          "codeExamples": ["fn greet(name: text) -> text { ... }"] }
      ]
    }
  ]
  ```
- Run automatically from `build.sh` (after `cargo run --bin gendoc`).

**Examples** (`assets/examples/`):
- Copy all `tests/docs/*.loft` files at build time.
- Register in `examples.js`:
  ```js
  export const EXAMPLES = [
    { id: '06-function', title: 'Functions',  file: 'examples/06-function.loft' },
    { id: '07-vector',   title: 'Vectors',    file: 'examples/07-vector.loft'  },
    // …
  ];
  ```

**JS deliverables**:
- `ide/src/docs.js` — renders the docs bundle; implements search (simple substring)
- `ide/src/examples.js` — loads `.loft` files; "Open as project" creates a new project
- Right-sidebar tabs: **Docs** | **Examples** | **Outline**

**Milestone check**: open the Docs tab, search for "filter"; click any code block
to copy it to the editor. Open the Examples tab, click "Vectors", hit Run.

---

### M6 — Export, Import & PWA

**Goal**: One-click ZIP download; import from ZIP; offline capability.

**JS deliverables**:
- `ide/src/export.js` (see API Contract for ZIP layout):
  - `exportZip(project)` — uses JSZip; generates `README.md` and shell scripts
  - `importZip(blob)` — reads ZIP, reconstructs project object
  - Drag-and-drop import anywhere on the window
- `ide/sw.js` — service worker:
  - Pre-caches `index.html`, `style.css`, all `src/*.js`, the WASM files, docs bundle, examples
  - Serves from cache when offline
- `ide/manifest.json` — PWA manifest (name, icons, start_url)
- URL sharing: single-file programs encoded as `#code=<base64>` in URL;
  decoded on load and opened in editor (no IndexedDB entry created)

**JS tests** (`tests/export.test.js`):
```js
test('exported ZIP contains main.loft', async () => {
  const project = {id:'z', name:'demo', modified:0,
                   files:[{name:'main.loft', content:'fn main(){}'},
                          {name:'util.loft', content:'fn add(a:integer)->integer{a+1}'}]};
  const blob = await exportZip(project);
  const zip  = await JSZip.loadAsync(blob);
  assert('demo/src/main.loft' in zip.files);
  assert('demo/src/util.loft' in zip.files);
  assert('demo/README.md'     in zip.files);
  assert('demo/run.sh'        in zip.files);
});

test('run.sh calls loft on main.loft', async () => {
  const project = {id:'r', name:'myapp', modified:0,
                   files:[{name:'main.loft', content:''}]};
  const blob = await exportZip(project);
  const zip  = await JSZip.loadAsync(blob);
  const sh   = await zip.files['myapp/run.sh'].async('string');
  assert(sh.includes('loft'));
  assert(sh.includes('src/main.loft'));
});

test('import roundtrip preserves content', async () => {
  const original = {id:'i', name:'test', modified:0,
                    files:[{name:'main.loft', content:'fn main(){println("x");}'}]};
  const blob    = await exportZip(original);
  const project = await importZip(blob);
  assert(project.files[0].content === 'fn main(){println("x");}');
});

test('URL share encodes and decodes', () => {
  const code    = 'fn main(){println("hi");}';
  const encoded = encodeForUrl(code);
  const decoded = decodeFromUrl(encoded);
  assert(decoded === code);
});
```

**Milestone check**: click Export on a 3-file project; unzip; run `loft src/main.loft`
locally; output matches the browser console.

---

## Testing Strategy

### In-browser runner (`tests/runner.html`)

No build tools required.  Loads each `*.test.js` as an ES module; outputs a
pass/fail table.  One-click to run all tests after opening `runner.html`.

Tiny harness (`tests/harness.js`):
```js
let pass = 0, fail = 0;
export function test(name, fn) {
  try { fn(); pass++; console.log('✓', name); }
  catch(e) { fail++; console.error('✗', name, e.message); }
}
export function assert(cond, msg='') {
  if (!cond) throw new Error(`Assertion failed${msg ? ': ' + msg : ''}`);
}
```

### Node.js (optional CI)

For the WASM integration tests, Node 18+ can load WASM:
```sh
node --experimental-vm-modules tests/wasm-bridge.test.js
```

The pure-JS tests (projects, export, symbols, language) run in Node without a
browser by mocking `indexedDB` with `fake-indexeddb` and `JSZip` directly.

---

## Export ZIP — Local Development Layout

```
<project-name>/
├── src/
│   ├── main.loft          (always present)
│   └── <other files>.loft
├── lib/
│   └── <library files>.loft  (if any)
├── README.md
│   # <project-name>
│   # Generated by Loft Web IDE
│   #
│   # Run locally:
│   #   loft src/main.loft
│   #
│   # Requirements:
│   #   https://github.com/<repo>/releases (loft binary)
├── run.sh   (chmod +x)
└── run.bat
```

The `lib/` folder maps to loft's `lib/<name>.loft` convention so `use <name>;`
statements resolve correctly after unzipping.  If no library files exist the
folder is omitted from the ZIP.

---

## Summary Roadmap

| Milestone | Focus | Key deliverable | Tests |
|---|---|---|---|
| M1 | WASM foundation | `wasm-bridge.js` + Rust wasm feature | 4 integration |
| M2 | Editor shell | `index.html` + CodeMirror + Loft grammar | 5 tokenizer |
| M3 | Navigation | `symbols.js` + go-to-def/find-usages | 3 unit |
| M4 | Multi-project | `projects.js` + file tree + tabs | 4 IndexedDB |
| M5 | Docs & examples | `docs.js` + `examples.js` + bundler script | — |
| M6 | Export & PWA | `export.js` + `sw.js` + ZIP format | 4 ZIP |

Each milestone is independently testable and deployable; later milestones do not
break earlier ones.  M1 and M2 can be developed in parallel once the Rust WASM
build produces a valid `.wasm` file.
