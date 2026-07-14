---
render_with_liquid: false
---
# What's new in loft

A short, friendly log of what has changed in each release.  Read top-to-bottom
for a tour of how the language has grown.

Looking for the deep technical history (opcode renames, slot allocator
invariants, internal phase numbers)?  See
[doc/claude/CHANGELOG_TECHNICAL.md](doc/claude/CHANGELOG_TECHNICAL.md).

---

## 2026-07

The **stability and type-safety** release. Two things anchor it: parsing text into a
number is now *honestly fallible* (it hands you a nullable instead of quietly
inventing a `0`), and a long-standing class of heap / memory bugs — leaks and
use-after-free corruption around returns, reassignment, and `match` — has been
retired wholesale and is now guarded on every night's CI. The registry, the sandbox,
and reference binding all move forward too.

### Patch `2026.7.1` — the downloadable binaries

`2026.7.0` shipped without its pre-built binaries: a release-pipeline bug published
the release before the platform builds could attach, and the published release was
immutable. `2026.7.1` is that same release with the four platform bundles actually
attached — Linux (`x86_64-musl`), macOS (Intel + Apple Silicon), and Windows — plus
the pipeline fix (binaries are now built into a draft *before* publish) so it cannot
recur. The loft binary itself is unchanged from `2026.7.0`.

### Parsing a number can fail — `text as integer` now gives you a nullable

This is the one change most existing code will need to look at.

- `"42" as integer` used to *always* hand back an integer, silently producing `0`
  when the text wasn't actually a number — a wrong answer that looked like a real
  one. Casting text to a number is now an honest **fallible parse**: `text as
  integer`, `text as float`, and `text as single` return a **nullable** (`integer?`,
  `float?`, `single?`), and yield `null` when the text isn't a valid number.
- `"42" as integer` is `42`; `"oops" as integer` is `null`.
- Handle the `null` at the cast site, most often with `??`:
  `count = field as integer ?? 0`, or keep the `integer?` and test it
  (`n = field as integer; if n { … }`).

> **Upgrading:** anywhere you wrote `x = some_text as integer` (or `as float` /
> `as single`) and then used `x` as a plain number, add a default —
> `x = some_text as integer ?? 0` — or give `x` a nullable type. The parse silently
> returning `0` on bad input is exactly the bug this closes, so the compiler now
> makes you say what should happen. `null as integer?` is how you write an explicit
> typed null.

### Float math that can't answer now hands back a nullable

The same honesty reaches floating-point arithmetic. A handful of operations have no
real answer for some inputs — `/` and `%` when the divisor is `0`, and `sqrt`, `ln`,
`log2`, `log10`, `asin`, `acos`, and `pow` outside their domain. Those now produce a
**nullable** (`float?` / `single?`) instead of a silent `NaN`, and the nullability
**propagates**: a value computed from a `float?` stays `float?` until you settle it.

- `sqrt(x)` and `a / b` are `float?`. Storing one into a spot that must hold a real
  number — a `-> float` return, a `not null` field — is where you discharge it:
  `sqrt(dx*dx + dy*dy) ?? 0.0`, or keep the `float?` and test it.
- A **literal operand known to be in range keeps the result non-null** — `x / 2.0`,
  `pow(x, 2.0)`, and `sqrt(2.0)` stay plain `float`, so most everyday arithmetic is
  untouched. `+`, `-`, and `*` on non-null floats are always `float`.
- This is a **warning, not an error**. Existing programs keep compiling and keep
  their old runtime behaviour; the compiler just marks each place a possibly-null
  float lands somewhere that expects a definite number.

> **Upgrading:** if a `-> float` function ends in a variable division or a
> `sqrt`/`pow`/`ln`/… of a variable, add a default at the boundary — `… ?? 0.0` — or
> widen the type to `float?`.

### A written compatibility contract — and a command to check it

loft now states its **compatibility contract** in writing
([COMPATIBILITY.md](doc/claude/COMPATIBILITY.md)): at contract level 1, a program
that runs today keeps running — the language, its error behaviour, and its libraries
all included. Both changes above fit inside it, which is exactly why the
nullable-parse and float-null shifts surface as **warnings** rather than breakage.

A new `loft api-surface` command makes the contract checkable. `loft api-surface
<file>` prints the public surface of a program or library — its `pub` functions,
types, sizes, and signatures — and `loft api-surface --diff <base> <new>` compares
two versions and reports a plain verdict (drop-in, or a break), exiting non-zero on a
break so a CI check can guard it.

### Error messages now point at the problem

Every error — parser, type, or runtime — now shows the file, line, and column,
the offending source line, and a caret under the exact token:

```
error: expected integer, got text on argument 2 of call to add
  --> game.loft:3:14
  |
3 |   x = add(1, "two");
  |              ^
```

- **Did-you-mean suggestions.** A misspelled variable, function, field, method,
  type, or enum variant suggests the near match — `unknown variant Color::Bleu —
  did you mean 'Blue'?`.
- **Concrete type mismatches** name both sides, the operation, and (for calls)
  the argument index — no more bare "type mismatch".
- **A mistyped `match` pattern** that can never match its subject (a text arm on
  an integer subject) is now a clear error instead of a silently-dead arm.
- Prefer the old single-line format? Set `LOFT_ERRORS=compact` (or pass
  `--errors=compact`).

Runtime faults (divide-by-zero, index out of bounds, null dereference,
narrowing-cast overflow) keep loft's rule of never aborting a running program:
they yield loft's usual sentinel (`null`, `0`, …) so a game or server keeps
running, and the fault is recorded with its source position instead of vanishing
silently.

### A whole class of memory bugs is gone

loft stores structs, vectors, and keyed collections on a managed heap. That heap had
a long tail of hard-to-see faults — a store leaked once per loop iteration, or a
value was freed while something still pointed at it (use-after-free) — clustered
around returning a value, reassigning one, and binding records out of `match` arms.
This release **retires that class**: whole-value binds copy and projections stay
views under one consistent rule, and the lifetime checker that decides when a store
is freed now reads a single source of truth instead of re-deriving it per site.

It stays fixed because three independent guards run continuously: a
poison-allocator test suite (every freed store is scribbled over, so any later read
is caught), an `Arbitrary`-driven program fuzzer, and a **nightly differential
oracle** that runs a growing corpus through *both* the interpreter and the `--native`
compiler and fails CI on any divergence in output, exit, or leak.

### Dense vectors and predictable copies

Vectors now default to dense storage, and the copy-versus-view model is spelled out
and enforced: a whole-value bind (`b = a`) **copies**, while a projection (`a.field`,
`v[i]` of a struct) stays a **view** onto the original. Narrowing casts are checked
rather than silently truncating.

### `&` — references you can write back through

A `&`-annotated binding creates a reference: pass `f(&x)` and the function can write
back to your `x`, or bind `a = &b` to alias a value for in-place update. It works on
scalars, heap values, and parameters, on both backends.

**Now including whole vectors.** A plain `d = v` (or `d = self.data`) *copies* the
vector — bind it, and the copy is independent. Write `d = &v` (or `d = &self.data`) and
you get a live, writable window instead: `d[i] = x` and `d += […]` write **through** to
the source. This is the ergonomic primitive for vector-heavy code — grab a sub-vector,
mutate it in place, no copy:

```loft
fn bump(self: Grid) {
  row = &self.cells;                 // a writable view of the field, not a copy
  for i in 0..len(row) { row[i] = row[i] + 1; }
}
```

The rule is one sentence now: **a plain bind copies; a `&` bind aliases** — the same for
scalars and vectors. (Reading a struct-typed field or element is still a view without
`&`, since it names an interior place, not a whole value.)

### Running untrusted code — the sandbox subset

A new compile-time **sandbox** lets you run untrusted loft with capability limits —
what a restricted caller may call, which parameters and fields it may touch, and what
it may mutate — enforced as a compile-time admission check, with an adversarial
escape suite proving the boundary holds.

### Games and the browser

`--html` gains **engine-less web modules** (a plain loft program compiles to a
self-contained WASM page), a `host_input()` primitive for feeding browser input in,
and asyncify resume that keeps running in headless / hidden tabs. A WebSocket WASM
bridge brings networked (and zero-trust crypto) programs to the browser.

### A pile of fixes

Among them: a keyed range or partial-key slice used as a value (`x = idx[lo..hi]`)
is now a clear compile error instead of a crash; `.map` on a literal receiver, a
nested-vector element-stride mismatch, and a native miscompile that returned an empty
vector from a struct field are all fixed. The four utility libraries touched by the
parse change — `arguments`, `random`, `regex`, and `cbor` — are migrated to the new
nullable-parse contract and republished.

## 2026-06

**New versioning.** Starting here, loft moves to a **monthly,
calendar-versioned cadence**: releases are named for their month — this one
is **`2026-06`** — which `Cargo.toml` spells `2026.6.0` (year.month.patch;
the patch digit is reserved for in-month security fixes). A deliberate step
up from the old `0.8.x` line.

This release rounds out loft's **library system** — toolchain-free native
libraries, signature-verified installs, and (with the namespace change below)
per-library namespaces instead of one shared flat space.

### Use a native library without a Rust toolchain

Native libraries (like `graphics` or `imaging`) used to compile their Rust
`cdylib` from source the first time you `use` them — needing `rustc`, `cargo`,
and the right system dev headers. loft can now fetch a **prebuilt** cdylib for
your platform and load it directly: no toolchain, no ~90-second first-use
compile. Building from source stays the automatic fallback when no prebuilt is
published for your platform.

### Registry installs are now signature-verified

`loft install` verifies the registry's index against a **trust root** embedded
in the loft binary before trusting any of it — every install is
cryptographically signed end to end, and a tampered index is refused. The trust
root is three independent keys, so a lost signing device can be retired without
disrupting anyone. Maintainers sign with a review-then-sign tool that shows
exactly what's going into a signature — re-downloading each library tarball to
confirm its checksum — before the key is ever used.

### Names that don't fight each other — enum-scoped variants, shadowing, import aliases

Naming got a lot less cramped (`@PLN22`):

- **Two enums can share a variant name.**  `enum Color { Red, Green }` and
  `enum Light { Red, Amber }` now coexist — a bare `Red` resolves from its
  context (the match subject, the declared type, a comparison, a function
  argument), and you can always qualify it as `Color.Red` when there's no
  context.  Defining a *new untyped variable* straight from a bare variant
  (`x = Red`) is a deliberate error — qualify it or give `x` a type — so adding a
  second enum with that variant can never silently re-point existing code.
- **Your names can shadow the standard library.**  `enum E`, `struct File`,
  `pub PI = 3` are all legal even though the stdlib already has `E` / `File` /
  `PI`; your definition wins bare lookup, and `std::E` still reaches the original.
  (The built-in *type* keywords — `integer`, `vector`, `iterator`, … — stay
  reserved.)
- **Import aliases.**  Rename a whole library or individual names on import:
  `use lib as m;` (qualifier `m::fn`), `use lib::Name as Alias;`, and grouped
  `use lib::(a as x, b, c);`.  Multiple names from one library must be
  parenthesised — `use lib::a, b;` is no longer accepted.

### Small integer types hold their full range — and won't silently null

The fixed-width integer types `u8`, `i8`, `u16`, `i16` pin a field to one or two
bytes; this release makes their ranges predictable and their edges safe.

- **`not null` gives the full native range.**  A `not null u16` now holds the
  whole `0..=65535` (before, `65535` read back as `null`); a `not null i8` holds
  `-128..=127` — exactly what the name promises.
- **A nullable field keeps one value aside for `null`**: `u8` is `0..=254` and
  `u16` `0..=65534` (the top trimmed), `i8` is `-127..=127` and `i16`
  `-32767..=32767` (kept symmetric).  Storing that one reserved value used to turn
  into `null` silently — now it's caught.  A literal is a compile error that tells
  you the fix (*"255 is reserved as the null sentinel of a nullable u8 (usable
  0..=254); declare the field `not null` for the full range, or cast with `as
  u8`"*); a value computed at run time gets a rate-limited warning that points you
  at the field while developing and stays quiet in a shipped game.
- **Narrow-element vectors match the fields.**  `vector<u16>` now holds `65535`
  and `vector<i16>` holds `32767`, just as `vector<u8>` already held `255`.

> Upgrading: if existing code stored the reserved edge value into a *nullable*
> narrow field (e.g. `255` into a `u8`), it will now flag instead of silently
> becoming `null` — declare the field `not null` for the full range, or cast.

### Windowed games without a server — `engine_host::run_local`

The games kernel gains a third way to run, next to the server (`run`) and the
network client (`run_client`): `run_local(tick_interval_us, on_event, on_tick)`
drives a **local windowed game** — steady ticks (one tick = one frame), the
kernel resting the CPU when nothing happens, and live build swaps — with no
server and no socket.  Close the window, call `client_stop()`, and the loop
returns.  When your game goes online later, you swap that one line for
`run_client` and keep your handlers exactly as they are.

### Window input as game events — `engine_host::post`

Post a local event from anywhere in your game — `engine_host::post("K:left")`
— and it arrives in your `on_event` handler like any network message
(`ev.cid == -1` tells you it came from this machine).  Key presses stop
slipping between frames, and your handlers no longer care whether input is
local or remote.  Servers with a window got their exit too: call
`engine_host::stop()` and `run` returns when the window closes.

### The debugger now tells you when a breakpoint can't work

Setting a breakpoint over `loft debug --rpc` answers with `verified` per
breakpoint: `false` means that line can never fire (no code on it, or a file
your program doesn't use) — so you find out immediately instead of waiting on
a stop that never comes.  Tracepoints also got friendlier: `"log": "expr"`
now works as a single expression (before, only the `["expr"]` array form did).

### An interactive prompt — `loft repl`

Run `loft` with no file (or `loft repl`) to get an interactive prompt where you
type loft one line at a time and see the result immediately:

```
loft> x = 40 + 2
loft> x
42
loft> fn dbl(n: integer) -> integer { n + n }
loft> dbl(x)
84
```

Names you bind stay available, functions and structs you define persist for the
session, multi-line input is supported, and a typo or run-time error doesn't end
the session.  Built-in commands inspect what you've defined — `:fns`, `:vars`
(each variable with its current value), `:bytecode`, `:rust`, `:slots` — and
`:help` lists them.

The prompt has **arrow-key history, in-line editing, and Tab completion** (of
function names, types, your variables, and `:`-commands), and it **remembers
your session**: the next time you start it, the variables and definitions from
last time are already there.  Start clean with `loft repl --fresh`.  See
[doc/claude/REPL.md](doc/claude/REPL.md).

### Look inside a program — `loft introspect`

`loft introspect <file>` prints a program's bytecode, the Rust loft generates
for it, per-function variable slot tables, and inferred types — side by side, in
one command.  Sub-flags pick one view (`--show-bytecode`, `--show-rust`, …) or a
single function (`--fn`).  This replaces hunting through `LOFT_LOG=…` dumps for
everyday inspection.

---

## 0.8.5 — 2026-06-07 — Language Maturity

This release is about the language itself getting solid.  Closures finally
work the way you'd expect, bounded generics carry types correctly through
methods and tuples, the native backend ships as production, and a
browser-based **branch review viewer** lets you read your in-flight code
from any device with `make view` + an SSH port-forward.

### Closures that capture what they should

The biggest user-visible fix.  Closures now hold a **live reference** to
captured variables, not a snapshot — they see the latest value, mutations
through one closure are visible to another, and a closure that captures a
struct field reads the field as it currently is.

```loft
counter = make_counter()  // returns a closure pair (inc, get)
counter.inc(); counter.inc(); counter.inc()
println(counter.get())    // 3 — was 1 before this release (snapshot bug)
```

- Closures returned from functions keep their captured environment alive.
- Multiple closures sharing the same captured cell see each other's
  writes.
- Closure-captured vector / struct / nested-struct fields read live, not
  stale.
- Validation matrix in `tests/closure_matrix.rs` cross-checks 30+ shapes
  on interpreter + `--native`.

### Bounded generics + interfaces

Write generic functions with type constraints; the compiler picks the
right per-type implementation at the call site.

```loft
fn show_pair<T: Printable>(a: T, b: T) -> text {
    "{a.to_text()} & {b.to_text()}"
}
println(show_pair(3, 7))           // 3 & 7   (built-in to_text)
println(show_pair("hi", "ho"))     // hi & ho (text passes through)
```

- `<T: Bound>` constraints — `Ordered`, `Equatable`, `Addable`, `Numeric`,
  `Scalable`, `Printable`, plus user-defined interfaces.
- Bound-typed values round-trip through tuples, vectors, struct fields,
  and `for` loops — the compiler now substitutes T's concrete type
  everywhere it appears.
- Generic functions returning `(T, T)` work with text, references, and
  user types — not just primitives.
- Format-string interpolation `"{x}"` where `x: T` routes through the
  bound's `to_text` method automatically.

### Tuples cross-validated end-to-end

Tuples now ship as a fully validated value type.  40 cross-mode test cells
cover 5 element types (scalars, text, nested tuples, closures, struct
references) across 3 storage destinations (local, direct stack, struct
field) — interpreter and `--native` produce byte-identical output.

```loft
fn split_message(s: text) -> (text, text) {
    n = s.len() / 2
    (s[0..n], s[n..s.len()])
}
left, right = split_message("hello world")
```

### Branch review viewer (`make view`)

`make view` launches a browser-accessible doc + code review surface for
the current loft branch.  Dashboard shows files changed vs `main`,
recent commits, uncommitted state — all with status badges.  Click any
file for a rendered view (`.md` rendered via the new `lib/markdown`
library, others as line-numbered code), toggle between
`Rendered ¦ Diff vs main`, click any commit for the per-file diff,
click any tracker tag (`@P-id` / `@PLAN-id`) for cross-doc references.
SSH-port-forward 8765 from the host.  Built entirely in loft (web
server, markdown rendering, JSON parsing, file walking) + a small bash
wrapper for `git` calls; no Python, no external markdown library, no
syntax-highlighter dependency.  See
[doc/claude/DEBUG.md § Branch review viewer](doc/claude/DEBUG.md#branch-review-viewer-make-view).

A `/welcome` landing page surfaces project status at a glance: open
problems, recently closed bugs (last 30 days), active and recently
finished plans, future plans by category — all built from a live tracker
index that updates on every commit.

### Tracker index (`make index`)

A small file-based index of every `@P<id>` / `@PLAN<id>(-segment)*`
reference across the project, queryable from the command line.  The
viewer surfaces the same data; CI uses it to catch broken tracker
references at commit time.

```bash
make index                            # rebuild index/tags.json
./scripts/idx tag:@P259               # all references to a P-issue
./scripts/idx prefix:@PLAN37          # all PLAN37-* phase refs
./scripts/idx incoming:doc/claude/PROBLEMS.md   # backlinks to a doc
./scripts/idx broken                  # broken @-references
./scripts/idx broken-links            # broken markdown links
```

A loft-native scanner port (`make index-loft`) reproduces the bash
scanner's output via the loft language itself — exercises long-running
file-walking + JSON emission shapes that no other loft program touches.

### Native compilation goes production

The `--native` backend (loft → Rust → rustc → standalone binary) is now
the default.  108 / 108 native tests pass; closures, generics, tuples,
JSON, and the viewer all compile + run identically under `--native` and
`--interpret`.  Use `--interpret` only when bisecting a native-only
regression.

Eight previously-tracked native codegen bugs closed (use-after-free in
heap-typed tail returns, text-concat type-dispatch, generic vector
struct returns, closure-tuple-field layout, parallel-queue native
runtime, and four more).

### `lib/markdown` — markdown renderer in loft

A standalone library: headings, bold, italic, inline code, fenced code,
links with anchor support, tables (with alignment), lists (ordered +
unordered + nested), images, autolinks for tracker tags, autolink
prefix configuration, image-URL rewriting.  Pure loft — no external
parser.  Used by the branch-review viewer and any future loft
documentation tool.

```loft
use markdown
html = markdown::render(source, "/tag/", "/img/", "")
```

### Smaller language wins

- **`@P274`** — `text + integer` concat now correctly converts the
  integer (was emitting `OpAppendText` with a raw i64; SIGSEGV in
  interp / E0614 in native).
- **`@P275`** — module-scope `const vector<T>` works under the
  default `--native` path (was only initialised under
  `--native-release`; default emit panicked at
  `stores.const_refs[NNN]`).  Side-fix: nested `OpConstRef` calls
  no longer accumulate `stor` prefixes in their substituted form
  (a substring-of-its-own-output bug in the codegen template
  rewriter).
- **`@P276`** — `(s[i] ?? '<c>') == '<c>'` now type-checks under
  `--native` (was rustc E0308: the pre-evaluated block holding
  the character lifted as `i32`, then the outer
  `OpConvIntFromCharacter` template compared it against `char`).
  Bind-then-compare (`c = s[i] ?? '*'; if c == 'b'`), else-if
  chains, and ordering compares (`<`/`>`) all work too.
- **`@P283`** — format-string interpolation of a self-slice-
  reassigned text PARAMETER no longer crashes either backend.
  Pattern: `fn f(rb: text, id: text) -> text { …; rb = rb[a..b];
  "[{id}] {rb}" }` was SIGSEGVing the interpreter and rejecting
  with rustc E0368 in native.  The work-buffer parameter
  promoted by `text_return` is `RefVar(Text)` (`&mut String`),
  but the codegen for `OpAppendText` / `OpClearText` /
  `OpFormatText` / `OpFormat{Int,Float,Single,Database}` /
  `OpAppendCharacter` on these targets emitted the local-String
  variants — interp treated the refvar slot as a `String` →
  SIGSEGV; native emitted `var += &*(…)` on `&mut String` →
  E0368.  Fix dispatches to the matching `Stack` variant for
  RefVar(Text) targets on both backends (mirrors the existing
  B7 OpAppendCharacter dispatch).
- **`@P259`-`@P261`** — closure / store-allocation / vector-field
  fixes (the closure-cell trio).
- **UTF-8** — `json_parse` now decodes 2/3/4-byte UTF-8 codepoints
  correctly (was widening byte-by-byte; `→` became `âââ`).
- **WebSocket binary frames** — `lib/server` exercises the binary
  path in production; multi-client games use it.
- Eight new P-issues filed from dogfood discovery (native codegen +
  parser quirks surfaced by writing real loft consumers); fixes
  scheduled across the next few releases.

### Workflow + project-management

- New `## Open work` sections in reference docs catalog
  enhancement opportunities discovered while building real consumers.
- DEVELOPMENT.md documents the "fix-on-the-spot vs canonical-home"
  workflow for handling discovered language gaps mid-feature work.
- Plan documentation reorganized: `plans/` for core-language work
  (capped at 2-3 active), `lib_plans/` for library work, `ROADMAP.md`
  as the prioritization view.
- Every PROBLEMS.md row now self-tags with `**@P<n>**` so the
  index unambiguously links each row to its references.

### Relative file paths are now program-relative — portable "program + assets" bundles

A relative file path — `file("assets/font.ttf")`, `read_file("data.bin")`,
`delete("out.tmp")` — now resolves against **the program's own directory** (the
source dir under `--interpret`, the executable's dir under `--native`), not the
process working directory.  An asset addressed relative to your program loads no
matter where the program is launched from:

```loft
f = file("assets/level1.dat");   // beside the program, wherever it runs from
```

This is what #255 needed: a bundled font worked from the source tree but vanished
under `--native` (which runs from a temp dir), because the path resolved against
the cwd.  **Absolute paths are never rewritten.**  Resolution is uniform across
`file()`, `exists()`, `read_file`/`write_file`, the `File` methods,
`delete`/`move`/`mkdir`, and image loads.

**CLI tools opt back into cwd** with a one-line file-top directive — a
*user-supplied* relative path then resolves against the working directory:

```loft
#cwd
fn main(args: vector<text>) { data = read_file(args[1]); }
```

Per-invocation, `LOFT_PATHS=program` / `LOFT_PATHS=cwd` overrides both.
`source_dir()` returns the anchor and now works under `--native` (was empty
before).

**Breaking change** — a program that read or wrote a relative path expecting the
*working directory* now needs `#cwd` at the top.  The in-tree corpus that did so
(13 file-I/O tests) was migrated in this release.

### Faster startup, automatically — the program cache is on by default

Running the same program again is now **~3× faster to start**: the first
run caches the fully-parsed program (the standard library, every library
you `use`, and your script) next to your other caches, and later runs of
the unchanged program skip parsing entirely.  It just works — no flag to
set.  If anything the program reads changes, the cache notices and
re-parses, so you never get a stale result.

- **Turn it off** with `LOFT_NO_CACHE=1` (e.g. for one-shot batch jobs
  where the first-run save isn't worth it).
- **Cap its size** with `LOFT_CACHE_MAX_MB` (default 512 MiB); the oldest
  bundles are evicted past the limit.
- It automatically stays **off inside `cargo run` / `cargo test`**, so
  building the compiler never serves a stale parse.

### File `+=` is now append-only — and `file.sync()` lets you flush

`f += value` now **appends** to the end of the file, matching how
`vector += [elem]` and `text += "more"` work on the other collection
types.  Earlier writes are preserved when you re-open the file:

```loft
{f = file("log.txt"); f += "first\n";  f.sync(); }
{f = file("log.txt"); f += "second\n"; f.sync(); }
{f = file("log.txt"); f += "third\n";  f.sync(); }
// Result: 19 bytes — "first\nsecond\nthird\n", not just "third\n".
```

Use `f.sync()` between log records or block boundaries to guarantee
the buffered bytes have landed on disk before the next write is
issued.  Returns `true` on success; on `Directory` / `NotExists` the
call short-circuits to `false`.

**Breaking change** — code that relied on `f += …` truncating the file
on first re-open now needs to call `f.set_file_size(0)` (or
`f#size = 0`) explicitly before the first write.  Updated call sites
in this release: `tools/audience-demo/single_port_server.loft`,
`lib/world/src/world.loft`, `lib/graphics/src/glb.loft`,
`scripts/build-playground-examples.loft`.  Explicit offsets via
`f#next = N` still overwrite at offset `N`, so the snapshot idiom
(fixed-slot headers, overwrite-in-place) keeps working.

### Interpreter no longer corrupts memory on deep recursion

The interpreter's value stack now grows on demand.  Previously it was
a fixed 8 KB buffer that never expanded, so a program that nested
function calls deeply enough (roughly 40+ frames carrying a handful of
locals) would silently write past the buffer and corrupt the heap —
usually surfacing as a confusing "double free or corruption" abort
*after* the program had finished printing its output.  Deeply
recursive interpreted programs now run correctly (the `--native`
backend was never affected, as it uses the real machine stack).

## 0.8.4 — 2026-04-24 — Awesome Brick Buster

This release focuses on **the web**: your loft programs can now fetch
URLs, serve HTTP, parse JSON, and even run entirely inside a browser tab.
The headline is **Brick Buster** — a full arcade game, paddle + ball +
powerups + music + levels + high score, that you can share with a friend
via a single URL.

### JSON — read and write structured data

```loft
v = json_parse("{\"name\":\"Alice\",\"age\":30}")
println(v.field("name").as_text())   // Alice
println(v.to_json_pretty())          // formatted output
```

- `json_parse(text)` turns JSON into a value you can explore.
- Bad input returns a null value instead of crashing.  Ask
  `json_errors()` what went wrong.
- Build JSON from code with `json_number`, `json_string`,
  `json_array`, `json_object`, ...
- Read it back with `field("key")`, `item(index)`, `len()`, `keys()`.
- `MyStruct.parse(json_value)` fills a struct from JSON in one line.

### HTTP — talk to the web

```loft
use web
r = http_get("https://example.com")
if r.ok() { println(r.body) }
```

- `http_get`, `http_post`, `http_put`, `http_delete` — straightforward
  blocking calls that return an `HttpResponse` with `.status`, `.body`,
  and `.ok()`.
- `..._h` variants accept custom headers: `http_get_h(url, ["Accept: application/json"])`.
- A simple HTTP **server** is also available: `for req in listen(8080) { respond(req, ...) }`.

### Lighting that actually lights

The 3D renderer's PBR shader now uses the light colours and intensity
you pass in.  Previously the `Light` struct was accepted by the
scene-graph but the shader ignored `color_r/g/b`, `intensity`, and all
point lights — every scene looked as if lit by a single neutral-white
directional.

- A directional light's `intensity` scales its contribution.
- A scene's first **point light** is now rendered (quadratic
  attenuation; no shadow yet).
- Goldens for five of the graphics examples are checked in as
  regression guards — a shader tweak that breaks lighting is caught by
  a pixel-diff test.

### Games in the browser

- **Brick Buster** — a complete arcade game (paddle, ball, powerups,
  music, levels, high score) that runs in your browser and on the
  desktop.  Try it at
  <https://loft-lang.org/loft/brick-buster.html>.
- **Graphics gallery** — 24 WebGL demos, from hello-triangle to
  physically-based rendering.
- `loft --html program.loft` produces a single folder you can drop on
  any static web host.

### Easier code, clearer errors

- `parallel { }` really runs in parallel now (one OS thread per arm).
- `x ?? return err` — one line instead of a two-line null check.
- `type Handler = fn(Request) -> Response` — name function and tuple
  types.
- Any type with `fn next(self) -> Item?` can be used in `for x in val`.
- When the interpreter hits a fatal error, it now tells you *which
  function and line* triggered it before exiting.

### A gentler language

- `integer` is now 64-bit everywhere.  Big numbers like
  `9_876_543_210` just work — no suffix required.
- The old `long` type and `33l` literal suffix are gone; use `integer`
  and `33`.
- Three crashes involving `match` on complex types are fixed —
  character interpolation, uneven match arms, and chained native calls
  no longer leak memory.

### Native editor & tooling

- **Native Moros editor** — a full OpenGL editor ships as a standalone
  app you can distribute without installing loft.
- `loft --dump file.loft` — show the compiled bytecode without running
  the program.  Handy when something compiles oddly.
- New test runner: `scripts/find_problems.sh --bg` runs the whole suite
  in the background; check in with `--peek` or `--wait`.

---

### Closures you can return

Functions that return a closure now work correctly, including when the
closure captures variables from the enclosing scope:

```loft
fn make_greeter(prefix: text) -> fn(text) -> text {
    |name| { "{prefix}, {name}!" }
}
hello = make_greeter("Hi")
println(hello("Ada"))   // Hi, Ada!
```

Capturing closures also work with `map` and `filter`:

```loft
factor = 10
big = map(nums, |x| { x * factor })
```

### Quality-of-life fixes

- **Typos stop compilation.**  `y = unknown_thing` now fails with a
  clear error instead of silently creating a garbage value.
- **`rev(vector)`** — you can now iterate a plain vector in reverse.
- **Format strings** — `"{n:<5}"` (left-align), `"{n:^5}"` (centre) and
  `"{f:.0}"` (zero decimals) all behave the way you'd expect.
- **File reading** — `file.lines()` now returns text after the last
  newline, not just full lines.
- **Sorted collections** — descending primary-key ranges return
  correct results in every mode.
- **Windows paths** — native compilation correctly escapes `\` in file
  paths.

### Faster programs

- The compiler does arithmetic at compile time where it can, so
  `[for i in 0..100 { i * 2 }]` becomes a ready-made vector instead of
  a loop.
- `par(...)` automatically picks a lighter, faster worker when your
  work doesn't need its own scratch memory — no syntax change.

### Better docs

- New pages on **pattern matching** and **format strings**.
- Expanded chapters on images, threading, and generics.
- 137-page PDF reference regenerated.

---

## 0.8.3 — 2026-03-27 — WebAssembly!

Loft now runs in the browser.  The playground at
<https://loft-lang.org/loft/playground.html> compiles and executes
loft programs entirely in your browser tab — no server involved.

Behind the scenes:

- A virtual in-memory filesystem for browser tests.
- Captured `println` output for the playground.
- A stable plugin protocol so native extensions (imaging, random, web)
  can be loaded at runtime.
- String-heavy programs are faster thanks to format-string
  pre-allocation.

---

## 0.8.2 — 2026-03-24

### Lambdas

Write throw-away functions inline:

```loft
doubled = map([1, 2, 3], |x| { x * 2 })
```

The short form `|x| { ... }` infers types from where you use it.  Use
the long form `fn(x: integer) -> integer { ... }` when you want them
explicit.

### Named arguments and defaults

```loft
fn connect(host: text, port: integer = 80, tls: boolean = true) { ... }

connect("localhost")                       // uses both defaults
connect("localhost", tls: false)           // skips port by name
```

### Native compilation

Ship your loft program as a real native binary:

- `loft --native file.loft` — compile and run via `rustc`.
- `loft --native-emit out.rs` — save the generated Rust source.
- `loft --native-wasm out.wasm` — compile to WebAssembly.

### JSON, computed fields, field constraints

- `"{value:j}"` serialises any struct to JSON.
- `Type.parse(json_text)` parses JSON back into a struct.
- `computed(expr)` fields are recalculated on every read, no storage
  needed: `area: float computed(PI * $.r * $.r)`.
- `assert(...)` clauses on struct fields validate every write.

### Small but welcome

- Workers started with `par(...)` can now return `text` and enum
  values, not just numbers.
- `fn` prefix dropped on function references: write `apply(double, 7)`,
  not `apply(fn double, 7)`.
- `pub` is now required to expose a definition to other files — this
  keeps your module boundaries tidy.

### Clearer errors

- Using `string` as a type suggests `text` instead of a generic error.
- Six common mistakes now come with a fix suggestion.
- Several crashes on unusual input have become proper error messages.

### Bug fixes

- `c + d` on two characters now produces text, not a crash.
- Empty vector literal `[]` as an argument no longer crashes.
- `v += other_vec` on text-bearing vectors no longer corrupts data.
- `map`, `filter`, and `reduce` no longer trip over their own internal
  slots.

---

## 0.8.0 — 2026-03-17

### Match expressions

Pattern-match enums, structs, and scalars:

```loft
match shape {
    Circle { r } => PI * pow(r, 2.0),
    Rect { w, h } => w * h,
}
```

- The compiler checks that you cover every case.
- Supports `North | South =>` (or-patterns), `if r > 0.0` (guards),
  `1..=9` (ranges), null patterns, character patterns, and full
  `{ ... }` block bodies.

### Formatter

- `loft --format file.loft` — format in place.
- `loft --format-check file.loft` — fails if not formatted; useful in
  CI.

### Imports

- `use mylib::*` — bring in everything.
- `use mylib::Point, add` — pick out just what you need.
- Local definitions always win over imported ones.

### Higher-order helpers

```loft
doubles = map(numbers, fn double)
evens   = filter(numbers, fn is_even)
total   = reduce(numbers, fn add, 0)
```

### Testing made easier

- `loft --tests file.loft::test_name` — run a single test.
- `loft --tests 'file.loft::{a,b}'` — run a selection.
- `loft --tests --native` — compile tests to a native binary first.

### New standard-library helpers

- `now()` — milliseconds since 1970.
- `ticks()` — microseconds since program start, monotonic.
- `mkdir(path)` / `mkdir_all(path)` — make directories.
- `vector.clear()` — empty a vector.

### Clearer warnings

- Division or modulo by a constant zero.
- Unused loop variables (silence with `for _i in ...`).
- Unreachable code after `return`, `break`, or `continue`.
- Redundant null checks on `not null` fields.

### Bug fixes

- `x << 0` and `x >> 0` now return `x` instead of null.
- `NaN != x` now returns `true` (it was wrongly `false`).
- `??` works correctly with floats.
- Using `if` as an expression without `else` is now a compile error
  rather than silently returning null.
- Assigning `null` to a struct field no longer crashes.
- `sorted[key] = null` and `hash[key] = null` remove the entry, as
  documented.

---

## 0.1.0 — 2026-03-15 — First release

The core language, in one place.

### Types and values

- **Static types with inference** — no type annotations on locals; the
  compiler figures out the type from the first assignment.
- **Null safety** — every value may be null unless declared `not
  null`; null propagates through arithmetic; `?? default` supplies a
  fallback.
- **Primitives** — `boolean`, `integer`, `long`, `float`, `single`,
  `character`, `text`.
- **Structs** — named records: `Point { x: 1.0, y: 2.0 }`.
- **Enums** — both plain enums and struct-enums (variants with fields
  and per-variant methods).

### Control flow

- `if`/`else`, `for`/`in`, `break`, `continue`, `return`.
- For-loop extras — inline filter (`for x in v if x > 0`), loop
  attributes (`x#first`, `x#count`, `x#index`), in-loop removal
  (`v#remove`).

### Working with collections

- `[for x in v { expr }]` — vector comprehensions.
- `vector<T>` (dynamic array), `sorted<T>` (ordered tree),
  `index<T>` (multi-key tree), `hash<T>` (hash table).

### Text and formatting

- `"Hello {name}, score: {score:.2}"` — string interpolation with
  format specifiers.

### Other

- **Parallel execution** — `for a in items par(b=worker(a), 4) { ... }`
  spreads the work across CPU cores.
- **File I/O** — read, write, seek, directory listing, PNG images.
- **Logging** — `log_info`, `log_warn`, `log_error` with source
  location and rate limiting.
- **Libraries** — `use mylib;` imports from `.loft` files.

---

## Version comparison links

- [Unreleased vs 2026-06](https://github.com/loft-lang/loft/compare/v2026.6.0...main)
- [2026-06 (2026.6.0)](https://github.com/loft-lang/loft/compare/v0.8.5...v2026.6.0)
- [0.8.5](https://github.com/loft-lang/loft/compare/v0.8.4...v0.8.5)
- [0.8.4](https://github.com/loft-lang/loft/compare/v0.8.3...v0.8.4)
- [0.8.3](https://github.com/loft-lang/loft/compare/v0.8.2...v0.8.3)
- [0.8.2](https://github.com/loft-lang/loft/compare/v0.8.0...v0.8.2)
- [0.8.0](https://github.com/loft-lang/loft/compare/v0.1.0...v0.8.0)
- [0.1.0](https://github.com/loft-lang/loft/releases/tag/v0.1.0)
