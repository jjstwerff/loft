# Compiler Pipeline

This document covers how loft source code is turned into executable bytecode: the lexer, the two-pass parser, the IR, type resolution, scope analysis, and bytecode generation.

---

## Contents
- [Pipeline overview](#pipeline-overview)
- [Lexer (`src/lexer.rs`)](#lexer-srclexerrs)
- [Parser (`src/parser/`)](#parser-srcparser)
- [IR — The `Value` tree (`src/data.rs`)](#ir--the-value-tree-srcdatars)
- [Type resolution (`src/typedef.rs`)](#type-resolution-srctypedefrs)
- [Scope analysis (`src/scopes.rs`)](#scope-analysis-srcscopesrs)
- [Rust code generation (`src/generation/`)](#rust-code-generation-srcgenerationrs)
- [Bytecode generation (`src/compile.rs`, `src/state/`)](#bytecode-generation-srccompilers-srcstate)
- [Default library (`default/*.loft`)](#default-library-defaultloft)
- [Naming conventions enforced by the parser](#naming-conventions-enforced-by-the-parser)
- [Diagnostic system (`src/diagnostics.rs`)](#diagnostic-system-srcdiagnosticsrs)
- [Source file summary](#source-file-summary)

---

## Pipeline overview

```
Source text (.loft)
       │
       ▼
  [ Lexer ]           src/lexer.rs
  tokenises chars into LexItem stream
       │
       ▼
  [ Parser — first pass ]     src/parser/
  defines all names; determines types; claims variables
  lenient: unknowns are allowed, deferred to pass 2
       │
       ▼
  [ typedef::actual_types ]   src/typedef.rs
  resolves all unknown types; fills Stores schema
       │
       ▼
  [ Parser — second pass ]    src/parser/
  generates IR (Value tree) with full type knowledge
       │
       ▼
  [ typedef::fill_all ]       src/typedef.rs
  finalises field positions in Stores
       │
       ▼
  [ enum_fn ]                 src/parser/definitions.rs
  synthesises polymorphic dispatch functions for enums
       │
       ▼
  [ scopes::check ]           src/scopes.rs
  assigns scope numbers to variables; inserts free/drop ops
       │
       ▼
  [ byte_code ]               src/compile.rs
  compiles IR Value trees → flat bytecode in State
       │
       ▼
  [ state.execute ]           src/state/mod.rs
  runs bytecode
```

---

## Lexer (`src/lexer.rs`)

### Core types

```rust
pub enum LexItem {
    Integer(u32, bool),  // value, started_with_zero
    Long(u64),
    Float(f64),
    Single(f32),
    Token(String),       // keyword or punctuation
    Identifier(String),  // any non-keyword identifier
    CString(String),     // string literal content up to next { or "
    Character(u32),      // 'x' character constant
    None,                // end of input / end of line
}
```

`LexResult` bundles a `LexItem` with a `Position` (file, line, column).

### Token and keyword sets

Defined as static slices at the top of the file:

- **TOKENS** — punctuation and multi-character operators:
  `:`, `::`, `.`, `..`, `,`, `{`, `}`, `(`, `)`, `[`, `]`, `;`, `!`, `!=`, `+`, `+=`, `-`, `-=`, `*`, `*=`, `/`, `/=`, `%`, `%=`, `=`, `==`, `<`, `<=`, `>`, `>=`, `&`, `&&`, `|`, `||`, `->`, `=>`, `^`, `<<`, `>>`, `$`, `//`, `#`

- **KEYWORDS** — reserved words that are emitted as `Token`, not `Identifier`:
  `as`, `if`, `in`, `else`, `for`, `continue`, `break`, `return`, `true`, `false`, `null`, `struct`, `fn`, `type`, `enum`, `pub`, `and`, `or`, `use`, `match`, `sizeof`, `debug_assert`, `assert`, `panic`

  Note: `fields` was temporarily in KEYWORDS (L3) but is removed in A10.0.  A10 uses
  `s#fields` postfix syntax, so no keyword reservation is needed.

  **Intrinsic keyword handling:**
  - `sizeof` — handled in `parse_single` via `has_token("sizeof")` → `parse_size`.
  - `assert` / `panic` — handled in `parse_single` via `has_token` → `parse_intrinsic_call`, which parses arguments and delegates to `parse_call_diagnostic` for file/line injection. These names are also defined as `pub fn` in `default/01_code.loft`; `parse_fn_name()` in `definitions.rs` allows keyword tokens as function names when `self.default` is true so that the default library can register their signatures.
  - `debug_assert` — reserved for A2.3; currently produces a parse error if used in user code.
  - `s#fields` — A10 field iteration; `fields` is recognized contextually after `#` in `parse_for`, not as a pre-reserved keyword.

  Names recognized by name in `parse_call` but intentionally left as identifiers (lower collision risk): `log_info`, `log_warn`, `log_error`, `log_fatal`, `parallel_for`, `fields`.

The lexer tries two-character tokens first (e.g. `!=` before `!`). Keywords are detected after the identifier is collected.

### Lexer modes

```rust
pub enum Mode {
    Code,        // normal code: skip whitespace and line endings
    Formatting,  // inside a format string after `{`: preserve spaces
}
```

Mode switches happen inside string scanning. When a `{` is encountered inside a string, the lexer switches to `Formatting` and returns the prefix as `CString`. The parser then reads a format expression. When `}` is encountered in `Formatting` mode, the lexer returns to scanning the rest of the string.

This allows inline format expressions like `"result: {value:>10}"` to be tokenised seamlessly.

### String literals and escape sequences

- `"..."` → `CString` for each segment between `{...}` format expressions.
- `\\`, `\"`, `\'`, `\t`, `\r`, `\n` are supported escape sequences.
- `{{` and `}}` inside strings are literal braces.

### Number literals

| Syntax | Result |
|---|---|
| `123` | `Integer(123, false)` |
| `0xaf` | `Integer(0xaf, false)` |
| `0b1010` | `Integer(10, false)` |
| `0o17` | `Integer(15, false)` |
| `123l` | `Long(123)` |
| `1.5` | `Float(1.5)` |
| `1.5f` | `Single(1.5)` |
| `1e2` | `Float(100.0)` |

Special case: `1..4` tokenises as `Integer(1)`, `Token("..")`, `Integer(4)` — the lexer uses a look-ahead to avoid consuming `..` as part of a float.

### Backtracking with `Link` / `revert`

The lexer supports arbitrary lookahead through a memory buffer:

```rust
let link = lexer.link();    // save current position; start buffering tokens
// ... try parsing something ...
lexer.revert(link);         // restore position; replay buffered tokens
```

`link()` increments a reference count. While any link is alive all consumed tokens are buffered. `Link` implements `Drop` to decrement the count; when the count reaches zero the buffer is discarded.

The parser uses this to speculatively attempt a parse path (e.g. checking whether an identifier is a type name or a variable) and backtrack on failure.

### Key lexer methods

| Method | Purpose |
|---|---|
| `cont()` | Advance to the next token (stored in `peek`) |
| `peek()` | Return the current token without advancing |
| `peek_token(s)` | Return true if current token equals `s` |
| `has_token(s)` | Consume and return true if current token equals `s` |
| `token(s)` | Consume expected token; emit error if not found |
| `has_identifier()` | Consume and return if current item is `Identifier` |
| `has_integer()` | Consume and return if current item is `Integer` |
| `has_cstring()` | Consume and return if current item is `CString` |
| `has_keyword(s)` | Consume if current item is `Identifier(s)` (local keyword) |
| `link()` / `revert(l)` | Save / restore lexer position |
| `switch(filename)` | Open a new file and restart |
| `parse_string(text, name)` | Switch to an in-memory string |

---

## Parser (`src/parser/`)

### `Parser` struct

```rust
pub struct Parser {
    pub data: Data,           // all definitions (functions, types, structs, enums)
    pub database: Stores,     // runtime type schema (field positions, sizes)
    pub lexer: Lexer,
    pub diagnostics: Diagnostics,
    first_pass: bool,         // true during first pass, false during second
    context: u32,             // definition number of the function being parsed
    vars: Function,           // variable table for the current function
    in_loop: bool,            // whether break/continue are valid
    default: bool,            // true when parsing the default/ library
    file: u32,
    line: u32,
}
```

### Two-pass design

Every source file is parsed **twice**:

**First pass** (`first_pass = true`):
- Registers all type, enum, struct, and function definitions.
- Assigns variable slots (but types may still be `Unknown`).
- Lenient: unknown types and unresolved names do not cause errors.
- Claims working text variables for string assembly expressions.
- Records which stores (via `database`) are mutated by each function.
- After the first pass, `typedef::actual_types` resolves unknown types and `typedef::fill_all` computes field offsets in `Stores`.

**Second pass** (`first_pass = false`):
- Generates the full `Value` IR tree for each function body.
- All type names, variable types, and function signatures must be known.
- Emits errors for type mismatches, unknown variables, and call failures.

The two-pass approach allows forward references — a struct or function can be used before it is defined.

### Entry points

```rust
// Parse a file (two full passes)
parser.parse("path/to/file.loft", is_default);

// Parse all .loft files in a directory, alphabetically
parser.parse_dir("default", true, debug);

// Parse from an in-memory string (used in tests)
parser.parse_str(text, "filename", logging);
```

`parse_dir` recurses into subdirectories and calls `scopes::check` after each file.

### `parse_file` — top-level loop

```rust
fn parse_file(&mut self) {
    // 1. Process `use` declarations first, switching the lexer to the
    //    included file and returning when it's done.
    while self.lexer.has_token("use") { ... }

    // 2. Parse top-level definitions in a loop:
    loop {
        self.lexer.has_token("pub");   // optional pub modifier
        if !parse_enum()
        && !parse_typedef()
        && !parse_function()
        && !parse_struct()
        && !parse_constant() { break; }
    }

    // 3. Resolve types and fill the Stores schema.
    typedef::actual_types(...);
    typedef::fill_all(...);
    database.finish();

    // 4. Synthesise polymorphic dispatch helpers.
    self.enum_fn();
}
```

### `use` resolution — `lib_path`

When `use foo;` is encountered, the parser looks for `foo.loft` in the following order:

1. `lib/foo.loft` (project-local library)
2. `foo.loft` (current directory)
3. `<current_dir>/lib/foo.loft`
4. `<base_dir>/lib/foo.loft` (when inside `tests/`)
5. Directories from the `LOFT_LIB` environment variable
6. `<current_dir>/foo.loft`
7. `<base_dir>/foo.loft`

### Operator precedence

Binary operators are parsed using a recursive-descent precedence climber. `OPERATORS` lists levels from lowest to highest precedence:

```rust
static OPERATORS: &[&[&str]] = &[
    &["||", "or"],                           // 0 — lowest
    &["&&", "and"],
    &["==", "!=", "<", "<=", ">", ">="],
    &["|"],
    &["^"],
    &["&"],
    &["<<", ">>"],
    &["-", "+"],
    &["*", "/", "%"],
    &["as"],                                 // 9 — highest
];
```

`parse_operators(precedence)` handles one level; it calls `parse_operators(precedence+1)` for the right operand. At the top of the recursion, `parse_part` handles postfix `.field` and `[index]` access, and `parse_single` handles atoms.

### `parse_single` — atom parsing

Handles the innermost syntactic unit:

| Token | Result |
|---|---|
| `!` / `-` | Unary not / negate |
| `(` expr `)` | Grouped expression |
| `{` block `}` | Inline block |
| `[` ... `]` | Vector literal |
| `if` | Inline if-expression |
| `fn` identifier | Compile-time function reference → `Value::Int(d_nr)` (see below) |
| identifier | Variable, function call, type constructor, or method |
| `$` | Current record reference (inside struct field defaults) |
| integer / long / float / single | Literal |
| string | Format-string expression |
| character | Character literal as integer |
| `true` / `false` / `null` | Literal boolean / null |

**Method call with same-type variable (`parse_single` Issue 1 fix):** When an identifier
resolves to a Reference-typed variable and the current parse context is an assignment
target of the same Reference type (i.e. `d = c.method()` where both `d` and `c` are the
same struct), `parse_single` calls `vars.make_independent(d, c)` (records that `d` is a
fresh copy of `c`'s slot) and returns `Value::Var(c)` directly. It does **not** emit
`OpCopyRecord(c, d, tp)` as the method self-argument, which was the root cause of Issue 1
(garbage `store_nr` crash). `generate_set` handles direct-assignment `d = c` via a
`ConvRefFromNull + Database + CopyRecord` sequence in its own branch.

### Function parsing — `parse_function`

```
'fn' name '(' [args] ')' ['->' return_type] ( ';' | '{' body '}' )
```

- First pass: registers the definition via `data.add_fn` or `data.add_op`.
- Second pass: looks up existing definition with `data.get_fn`, parses body, stores the code in `data.definitions[context].code`.
- Functions ending with `;` have no body (declaration of an external/built-in operation).
- After the body, `parse_rust` optionally reads `#rust "..."` annotations for the code generator.

**Important — internal function naming:**
- `add_fn` stores user-defined functions under the key `"n_<name>"` (e.g. `fn helper` → `"n_helper"`), not under `"helper"`.
- `add_op` (used only for default-library operators) stores under the plain name.
- `def_nr("helper")` therefore returns `u32::MAX` even if `fn helper` exists — the name `"helper"` is not in `def_names`.
- Consequence for type resolution: if a user writes `v: helper` (function name used as a type), `parse_type("helper")` sees `u32::MAX`, creates a `DefType::Unknown` entry for `"helper"` on the first pass, and `actual_types` emits "Undefined type helper" after the first pass. This is the correct/expected error path — no second-pass diagnostic needed.

### Struct parsing — `parse_struct`

```
'struct' Name '{' field* '}'
```

Each field: `name ':' type ['=' default] ['limit' min '..' max] [CHECK(...)]`

- Field types with `default(expr)` or `virtual(expr)` are handled via `parse_field_default`.
- `$` in a default expression is replaced by `Value::Var(0)` (the current record reference) at struct-init time.
- Trailing commas are allowed.

### Enum parsing — `parse_enum`

```
'enum' Name '{' variant* '}'
```

Two forms of variant:
- Plain: `Name` — a simple value.
- Struct-enum: `Name '{' field* '}'` — a variant with fields (polymorphic record).

After parsing, `enum_fn` synthesises dynamic dispatch wrappers so that functions defined on specific variants can be called polymorphically.

**`enum_fn` / `enum_numbers` — text-buffer forwarding (2026-03-13):**
`enum_fn` runs at the END of the **first pass**, immediately after all variant struct
types are registered. At that point `text_return` has already added `RefVar(Text)`
attributes to each variant function (because `text_return` runs during `parse_code` →
`block_result` for the function body, which is second-pass-only, so the attributes ARE
present by the time `enum_fn` runs in the *first* pass when types are complete).

To forward text-buffer arguments from the dispatcher to each variant:
1. `enum_fn` iterates `args[1..]` (all attributes beyond `self`) and creates a
   corresponding dispatcher argument for each; for `RefVar(Text)` attributes the
   variable is registered with `become_argument`.
2. `extra_call_args` and `extra_call_types` are collected from the dispatcher's own
   variable table for each such attribute.
3. `enum_numbers` is called with these extras; each variant's call IR becomes
   `Call(describe_Variant, [Var(0), Var(dispatcher_buf)])` instead of
   `Call(describe_Variant, [Var(0)])`.

**`generate_call` — `RefVar` forwarding special case (2026-03-13):**
When compiling a mutable argument whose type is `RefVar(_)` and the parameter is
`Var(v)` with `v` also typed `RefVar(_)`, emit only `OpVarRef(var_pos)` (reads the raw
`DbRef`) instead of the usual `generate_var` path which adds `OpGetStackText` after
`OpVarRef`.  The dereference (`OpGetStackText`) must be suppressed when the callee
expects a `DbRef` pointer, not the `str` content.

### ~~`parse_append_vector` — `RefVar(Vector)` gap~~ **FIXED (Issue 4)**

Previously, `v += items` inside a `&vector<T>` parameter was silently discarded.
The fix is `assign_refvar_vector` in `parse_assign` (see the `parse_assign` section
above). The old `parse_append_vector` path (used for non-RefVar vectors) is unchanged.

### Type parsing — `parse_type`

Converts a type identifier into a `Type` enum value. Handles:
- Built-in types: `integer`, `long`, `float`, `single`, `boolean`, `text`, `character`, `reference`
- Generic containers: `vector<T>`, `sorted<T[key]>`, `index<T[key]>`, `hash<T[key]>`
- User-defined structs and enums by name lookup
- `&T` reference types

### Type conversion and casting — `convert` and `cast`

Before emitting a binary operation or assignment, the parser checks if the actual type is compatible with the expected type:

1. **`convert`** — implicit, lossless conversion (e.g. widening an integer range, converting null, unwrapping a `RefVar`). Looks for `OpConv*` operators.
2. **`cast`** — explicit `as` conversion (e.g. text to enum, int to enum). Looks for `OpCast*` operators.
3. **`can_convert`** — pure check used for error reporting without code modification.

### String and format expression parsing — `parse_string`

When the lexer emits a `CString` followed by format mode:

```
"prefix {expr [:format_spec]} suffix"
```

The parser builds an `Insert` or append sequence:
- The prefix string literal is emitted.
- The format expression is parsed as a normal expression via `expression()`.
- A format specifier (width, radix, alignment, padding) is parsed by `string_states` and `get_radix`.
- The corresponding `OpFormat*` operator is called.
- The suffix string literal is emitted.
- The whole thing is assembled into text using append operations.

`expression()` internally calls `known_var_or_type` which emits an "Unknown variable" error
if the expression variable has not yet been assigned (i.e., `is_defined == false`). This is
the diagnostic path for PROBLEMS #10 — using `{cd}` before `cd = val` in the source.  The
`is_defined` flag is now correctly maintained: it is set only when the `=` token is
confirmed in `parse_assign`, not speculatively beforehand.

Loop-counter variables like `e#count` and `e#first` (lazily created in `iter_op`) are
explicitly marked defined when first referenced, so they are always valid inside a loop body.

### Variable tracking — `Function` / `vars`

`self.vars` (a `Function` from `src/variables/`) tracks the variable table for the function being compiled:

- `create_var(name, type)` — allocates a new slot.
- `unique(prefix, type)` — allocates an anonymous working variable.
- `change_var_type(nr, type)` — updates the inferred type of a variable.
- `become_argument(nr)` — marks a slot as a function parameter.
- `work_texts()` — returns slots claimed for text assembly.
- `test_used(lexer, data)` — emits warnings for unused variables.

### Vector literal parsing — `parse_vector` / `vector_db`

`parse_vector` (called when `[` is encountered) builds vector literal and append IR. It internally tracks the "owner variable" slot `vec` as a `u16`:

- If parsing an append to a struct field (`is_field = true`), `vec = u16::MAX` (sentinel meaning "no owning variable").
- If parsing a plain variable append (`v += [...]`), `vec = variable_slot_number`.
- Otherwise a temporary slot is created via `create_unique`.

`vector_db` (called from `build_vector_list`) emits the `OpDatabase` op that allocates a store for new struct-valued vector elements. It must guard against `vec == u16::MAX` before calling `is_argument(vec)`:

```rust
fn vector_db(&mut self, assign_tp: &Type, vec: u16) -> Vec<Value> {
    if self.first_pass || vec == u16::MAX || self.vars.is_argument(vec) {
        Vec::new()  // skip: field context, first pass, or function argument
    } else { ... }
}
```

Without the `vec == u16::MAX` guard, calling `is_argument(u16::MAX)` would panic with an out-of-bounds index (since `u16::MAX = 65535` far exceeds the variable table size). This was a bug that triggered whenever a `vector<Struct>` field was appended to using a struct literal, e.g. `q.list += [Num{v:1}, Num{v:2}]`.

---

### Runtime safety checks in the second pass

During the second pass, `parse_assign` and `parse_function` enforce two additional
safety invariants beyond type-checking:

**For-loop mutation guard (`parse_assign`, `variables/`):**
When parsing `v += items`, if the type is a collection (`Vector`, `Sorted`, `Index`, or
`Spacial`) and `v` resolves to a `Value::Var(v_nr)`, the parser calls `vars.is_iterated_var(v_nr)`.
This walks the `current_loop` chain in `variables/` comparing against each loop's `coll_var`
(original collection variable, set via `set_coll_var()` in `parse_for`). If the variable is
currently being iterated, a compile error is emitted:

```
Cannot add elements to 'v' while it is being iterated — use a separate collection or add after the loop
```

The check only fires for `Value::Var` LHS, not field access (`Value::Field`), so `db.items += x`
is not blocked. `v#remove` in a filtered loop is explicitly allowed — it is implemented via
`OpRemove` which adjusts the iterator position before removing.

**Empty-body stubs (`parse_function`, `def_code` in `state.rs`):**
A function whose body is an empty block `{ }` AND whose first parameter is named `self` is
treated as an intentional polymorphic stub. Two effects:
- `parse_function` skips the `test_used` call that would emit "Parameter self is never read".
- `def_code` detects the empty `Value::Block` case, performs normal argument claiming (so that
  owned references like Text/Reference get their lifecycle managed correctly), then emits only
  `OpReturn` — the stub silently returns null for its declared return type.

Detection requires the first parameter to be named `self` to avoid false positives on ordinary
empty helper functions like `fn setup() { }`.

---

### `parse_assign` — assignment and mutating operators

```
expr [ '=' | '+=' | '-=' | '*=' | '%=' | '/=' ] expr
```

For a simple `=`:
- If the left side is a variable, the right side type is used to refine the variable's type (`change_var_type`).
- If the left side is a field, `set_field` emits the appropriate `OpSet*` call.

**`vars.defined` placement (PROBLEMS #10 fix, 2026-03-15):** `vars.defined(v_nr)` is
called *inside* the `has_token("=")` block, only after the `=` token has been confirmed.
Before this fix, the call preceded the token check, so any bare `Value::Var` seen as a
candidate LHS (including `{cd}` inside a format expression) was incorrectly marked
defined, hiding the "use before assignment" error and causing a panic in the
byte-code generator when the variable's stack slot was still `u16::MAX`.

For `+=` on text: delegates to `assign_text` which manages the string-assembly working variable.

For `+=` on `&vector<T>` parameters: handled by `assign_refvar_vector`. When the LHS variable has type `RefVar(Vector)` and the operator is `+=`, and the RHS is not a `Value::Insert` or `Value::Block` (bracket-form literals/comprehensions), it emits `OpAppendVector(Var(v_nr), rhs_expr, rec_tp)`. Bracket-form `[elem]` and vector comprehensions produce `Value::Insert` / `Value::Block` on the RHS; those fall through to the existing `parse_block` expansion path which uses `OpFinishRecord` and already handles ref-params correctly.

The key implementation detail: a `&vector<T>` parameter is passed via `OpCreateStack`, which stores the caller's actual vector `DbRef` in field 0 of the temp record. `generate_var` for `RefVar(Vector)` emits `OpVarRef + OpGetStackRef(0)` — this correctly retrieves the caller's vector. `OpAppendVector` then appends to that vector in place, so the caller sees the change.

`find_written_vars` already recognises `OpAppendVector` as a write (via a pre-existing check on the opcode name), so the "Parameter 'v' has & but is never modified; remove the &" error is suppressed correctly.

---

### Function references — `parse_fn_ref`

The `fn <name>` atom expression (parsed by `parse_fn_ref`) produces a compile-time
integer containing the definition number of the named function:

```loft
fn double_score(r: const Score) -> integer { r.value * 2 }

// User-facing syntax — the parser rewrites this into an internal parallel_for call:
for a in items par(b=double_score(a), 4) { results += [b] }
```

The `fn` reference is lowered to `Value::Int(d_nr)` where `d_nr` is the definition index.
At bytecode generation this becomes `ConstInt(d_nr)`. The internal `parallel_for` native
function receives this integer and uses it to dispatch the worker. Users must not call
`parallel_for` directly; use the `par(...)` for-loop clause instead.

**Callable fn-ref variables (T1-1):** A variable or parameter of type `Type::Function` can
also be called directly via normal call syntax. `parse_call` checks whether the callee name
resolves to a local variable of `Type::Function`; if so, it emits `Value::CallRef(var_nr,
args)` instead of `Value::Call`. `generate_call_ref` in `state.rs` pushes the arguments,
computes `fn_var_dist = stack.position - var.stack_pos`, and emits
`OpCallRef | fn_var_dist: u16 | arg_size: u16` (op_code 252, declared in
`default/02_images.loft`). At runtime, `fn_call_ref` reads the `d_nr` from the variable,
looks it up in `fn_positions: Vec<u32>` (populated at the start of each execute call), and
dispatches via `fn_call`.

`fn(T) -> R` is a valid parameter type parsed by `parse_fn_type`:
```loft
fn apply(f: fn(integer) -> integer, x: integer) -> integer { f(x) }
```

`generate_var` handles `Type::Function` by emitting `OpVarInt` (fn refs are stored as `i32`
d_nr). `find_fn` in `data.rs` returns early to the `n_` global lookup when
`type_def_nr` returns `u32::MAX`, preventing a panic on `Function`-typed method dispatch.

### Reverse collection iteration — `rev(sorted_col)`

`parse_in_range()` recognises `rev(<expr>)` with no `..` when the expression type is
`Sorted` or `Index`.  It sets `Parser::reverse_iterator = true` and consumes the closing `)`.
`fill_iter()` checks this flag and ORs bit 64 into the `on` byte of the OpIterate/OpStep
instruction pair.  The flag is reset after both `fill_iter` calls inside `iterator()`, and
also on the first-pass early return (so it does not persist across parse passes).

At runtime, `state::step()` for type-2 (sorted) detects `on & 64` and calls
`vector::vector_step_rev()` instead of `vector_step()`. `vector_step_rev` treats any
position `>= length` (the value produced by `iterate()` for the "not started" sentinel)
as "start at the last element", then decrements on each call, and returns `i32::MAX`
when the beginning has been passed.

### Vector comprehension machinery — `build_comprehension_code`

`[for elm in v { body }]` in an array context compiles through `parse_vector_for` →
`build_comprehension_code`. The same infrastructure is used by `map`, `filter`, and `reduce`
(T1-3, done 2026-03-15).

**Key helper functions:**

| Function | Purpose |
|---|---|
| `for_type(in_type)` | Returns the loop-variable type for an iterable (`vector<T>` → `T`) |
| `iterator(code, in_type, it, iter_var, pre_var)` | Modifies `code` in place to become the iterator-init expression (`v_set(iter_var, -1)` for vectors); returns the per-step expression that reads the next element |
| `unique_elm_var(parent_tp, assign_tp, vec)` | Creates a `Reference`-typed temp variable used as the record slot for each appended element |
| `vector_db(elem_type, vec_var)` | Returns IR ops that allocate a new database store and initialize `vec_var` to point to it |
| `build_comprehension_code(vec, elm, in_t, in_type, var_tp, for_var, for_next, pre_var, fill, create_iter, if_step, body, val, is_var, is_field, block, tp)` | Builds the full loop IR: optionally calls `vector_db`, then `fill`, then `create_iter`, then a `v_loop` containing `for_next + null-break + optional-if_step + body + OpNewRecord + set_field + OpFinishRecord` |

**Key parameters to `build_comprehension_code`:**
- `vec` — result vector variable number
- `elm` — reference variable that receives each newly allocated slot
- `in_t` — element type of the result vector (mutable; updated by the function)
- `in_type` — type of the input collection (with `depending(vec_copy_var)` already applied)
- `var_tp` — type of the loop variable (from `for_type`)
- `for_next` — `v_set(for_var, iter_next)` — assigns next element to loop variable
- `if_step` — optional filter condition: `Value::Null` = no filter; non-Null = skip element when false
- `body` — expression whose value is appended to the result vector each iteration
- `is_var=false, is_field=false, block=true` — standalone result vector (include `vector_db`, push `Value::Var(result_vec)` at end). After `vector_db`, `tp` is updated to carry the db dependency so scopes does not emit a double `OpFreeRef` for the result variable.

**Iteration pattern for `Type::Vector` (from `iterator()`):**
- `create_iter` = `v_set(iter_var, -1)` — initialize index to -1
- `iter_next` = `v_block([v_set(iter_var, iter_var + 1), OpGetVector(vec, size, iter_var)])` — increment then read
- Null check in loop: convert element to bool → if false → `Value::Break(0)`

### Parallel for-loop — `parse_parallel_for_loop`

The `par(b=<worker_call>, <threads>)` clause on a `for` loop runs a worker function on
every element of a vector in parallel and delivers results in the original order:

```loft
for a in items par(b=my_func(a), 4) { sum += b; }   // global fn
for a in items par(b=a.my_method(), 4) { sum += b; } // method
```

The parser intercepts a `for … in … par(…) { … }` pattern in `parse_for`. When the
`par(` token is found after the range expression, it calls `parse_parallel_for_loop`,
which:

1. Parses the worker call expression (either `fn(elem)` or `elem.method()`) via
   `parse_parallel_worker` to extract `(fn_d_nr, return_type)`.
2. Infers `elem_size` from the element type's Stores byte size.
3. Infers `return_size` from the primitive return type (1 for bool, 4 for int/single,
   8 for float/long).
4. Rewrites the loop into:
   - `par_results = parallel_for(input, elem_size, return_size, threads, fn_d_nr)`
   - A conventional for-loop over the result vector that binds `b` to each element.

The worker function must take a single `const` reference argument of the element type
and return one primitive value (integer, float, single, long, or boolean). Text and
reference return types are not yet supported.

The native function `n_parallel_for` in `native.rs` calls `run_parallel_raw` in
`parallel.rs`, which spawns threads using Rayon and collects results in order.

---

## IR — The `Value` tree (`src/data.rs`)

The parser produces a tree of `Value` nodes that represents a function body.

### `Value` enum

IR node variants (full definition in [INTERMEDIATE.md](INTERMEDIATE.md)):
- Literals: `Null`, `Int(i32)`, `Long(i64)`, `Float(f64)`, `Single(f32)`, `Boolean(bool)`, `Text(String)`, `Enum(u8, u16)`
- Variables: `Var(u16)` (read), `Set(u16, Box<Value>)` (write)
- Calls: `Call(u32, Vec<Value>)` — definition nr + args; `CallRef(u16, Vec<Value>)` — fn-ref variable nr + args (see [Function references](#function-references--parse_fn_ref))
- Control: `Block`, `Insert`, `If`, `Loop`, `Break(u16)`, `Continue(u16)`, `Return`, `Drop`
- Iteration: `Iter(u16, init, step)`, `Keys(Vec<Key>)`

`Block` wraps a `Vec<Value>` (statement list), the result `Type`, a `scope` number, and a name used in bytecode dumps.

### `v_block`, `v_set`, `v_if`, `v_loop` — IR constructors

Convenience functions used throughout the parser:

```rust
v_block(ops, result_type, name) → Value::Block(...)
v_set(var, expr)               → Value::Insert([Value::Set(var, expr)])
v_if(cond, then, else)         → Value::If(...)
```

### `Type` enum

Carries the static type of a `Value`. Key variants:

| Variant | Meaning |
|---|---|
| `Unknown(u32)` | Not yet resolved (first pass, or pending inference) |
| `Null` | The null/absent value |
| `Void` | No return value |
| `Integer(min, max)` | Bounded integer; min/max drive storage size (1/2/4 bytes) |
| `Boolean` | True/false |
| `Long` | 64-bit integer |
| `Float` | 64-bit float |
| `Single` | 32-bit float |
| `Character` | Unicode code point (stored as `Int`) |
| `Text(Vec<u16>)` | String; the `Vec<u16>` lists variables this text depends on |
| `Enum(def_nr, is_ref, deps)` | Enum type; `is_ref` true for struct-enum references |
| `Reference(def_nr, deps)` | Record reference (pointer into a Store) |
| `Vector(Box<Type>, deps)` | Dynamic array |
| `Sorted/Index/Hash/Spacial` | Keyed collections |
| `RefVar(Box<Type>)` | Stack reference (`&T` parameter) |
| `Iterator(result, state)` | Iterator type |
| `Function(Vec<Type>, Box<Type>)` | First-class function type (arg types + return type); runtime value is `i32` d_nr; variables of this type are callable via normal call syntax |
| `Rewritten(Box<Type>)` | Marker that text/vector append was rewritten |

The dependency lists (`deps: Vec<u16>`) track which variables a reference-typed value "depends on" for lifetime purposes, used by scope analysis.

#### `Type::depend()` and `Type::depending()`

`depend() -> Vec<u16>` extracts the full dep list from any type, recursing through `RefVar`.

`depending(on: u16) -> Type` returns a copy of the type with `on` prepended to the dep list. Called during expression parsing whenever a compound value borrows storage from a local variable (e.g. a text value built from variable 3 → `Type::Text(vec![3])`).

#### `Type::RefVar`

`Type::RefVar(Box<Type>)` means "stack reference" — a DbRef pointing into the stack allocation of another variable, rather than an independently-owned record in a Store. It is used for `&text` parameters (function arguments that alias a caller's text variable). `depend()` on `RefVar` delegates to the inner type.

#### Text return dependencies

When a function returns `Type::Text`, `text_return()` in `parser.rs` promotes local text variables to function *attributes* of type `RefVar(Text)`, and lists the resulting attribute indices in the return type's dep vec. This means a returned text value keeps the caller's stack alive until the return value is consumed.

### `DefType` — definition categories

```rust
pub enum DefType {
    Unknown,     // not yet resolved
    Function,    // normal function
    Dynamic,     // polymorphic dispatch wrapper
    Enum,        // enum type
    EnumValue,   // one variant of an enum
    Struct,      // struct type
    Vector,      // vector type definition
    Type,        // built-in type (integer, text, …)
    Constant,    // named constant
}
```

### `Data` — the definition table

`Data` holds `Vec<Definition>` for every named entity. A `Definition` stores:
- `name`, `def_type`, `returned` (return type for functions)
- `attributes: Vec<Attribute>` — fields (for structs/enums) or parameters (for functions)
- `code: Value` — the compiled IR body
- `variables: Function` — the variable table
- `known_type: u16` — the corresponding `Stores` database type id
- `rust: String` — optional hand-written Rust body for built-in ops

Key `Data` methods:

| Method | Purpose |
|---|---|
| `def_nr(name)` | Look up definition index by name |
| `find_fn(source, name, type)` | Find function by name and first-argument type |
| `add_fn / add_op` | Register a new function/operator in first pass |
| `get_fn` | Find existing function in second pass |
| `get_possible(prefix, lexer)` | Get all definitions whose name starts with prefix |
| `definitions()` | Current count of definitions |
| `def(nr)` | Borrow a definition by index |

---

## Type resolution (`src/typedef.rs`)

Called after each parse pass inside `parse_file`:

### `actual_types`

Iterates all definitions added since `start_def` and:
- Resolves `Unknown` types to their concrete forms (now that all names are registered).
- For each struct/enum, calls `fill_database` to register fields in `Stores`.
- Ensures that vector-of-struct types are registered in `Stores`.

### `fill_database`

For a struct or enum definition, calls `Stores` methods to build the runtime type schema:
- `db.structure(name, parent)` — creates a record type.
- `db.field(s, name, type_id)` — adds a field.
- `db.enumerate(name)` + `db.value(e, variant, ...)` — creates an enum type.
- Field sizes (1/2/4 bytes for integers; 4 for references/vectors; 8 for long/float) are determined by `Type::size`.

### `fill_all`

Calls `database.finish()` to compute final field byte offsets for all record types.

---

## Scope analysis (`src/scopes.rs`)

`scopes::check(data)` is called after parsing a file. It visits every function's IR tree and:

1. Assigns each variable declaration to a scope number (0 = function arguments, 1 = function body, 2+ = nested blocks).
2. Tracks which scopes are currently open via a scope stack.
3. When a scope closes, inserts `OpFreeText` / `OpFreeRef` cleanup calls for variables that go out of scope.
4. Detects re-use of a variable name across sibling scopes and remaps the second occurrence to a fresh slot via `copy_variable`.

The scope numbers are written back into `Function.variables[i].scope` after the pass.

### Key data structures

- `var_scope: BTreeMap<u16, u16>` — maps variable number → scope number where it was first assigned.
- `var_mapping: HashMap<u16, u16>` — maps an original variable to its locally-copied replacement when a variable from an outer (exited) scope is reused in an inner scope.

### Variable assignment (`scan` on `Value::Set`)

When `Value::Set(v, value)` is processed:

1. If `v` already has a `var_scope` entry from a scope that is **no longer open** (not in the scope stack) and no mapping yet exists → call `copy_variable(v)` to create a fresh slot, and record the mapping.
2. For every variable index `d` in `function.tp(v).depend()` that is not yet in `var_scope` → insert `d` into `var_scope` at the current scope and prepend a null/empty initializer for `d` into the output as a `Value::Insert`.
3. Insert `v` into `var_scope` at the current scope (if not already present).

This ensures dependency variables are always initialised in the same scope as the variable that borrows them.

### Cleanup generation (`get_free_vars` / `free_vars`)

`get_free_vars(function, data, to_scope, tp, ret_var)` produces the `OpFree*` calls for all variables in `var_scope` up to `to_scope`:

```
for each variable v in scope:
    skip if v == ret_var  (it is being returned)

    if type is Text(_)
        → emit OpFreeText(v)

    if type is Reference/Vector/Enum(ref)
       AND dep list is empty        ← variable owns its allocation
       AND v ∉ tp.depend()          ← not needed by the return value
        → emit OpFreeRef(v)
```

`free_vars` then inserts the free ops into the IR:
- If the final expression is a `Value::Block`, free ops are inserted **inside** the block just before the block's last operator (`insert_free`), so cleanup runs before the block's `OpFreeStack`.
- Otherwise, free ops are inserted before or after the expression in the statement list.

### Block returns and `OpFreeStack`

`OpFreeStack(value_bytes, discard_bytes)` collapses a block's stack frame:
- decrements `stack_pos` by `discard_bytes`
- asserts no `text_positions` entries remain in the discarded range (debug builds)
- bitwise-copies `value_bytes` bytes as the block's result

**Constraint**: all `String`-typed (text) variables allocated **inside** a block must be freed with `OpFreeText` before `OpFreeStack` runs. The one exception is the *return variable* (`ret_var`), which is skipped in `get_free_vars`. This works safely only when the text variable was allocated **outside** the block (function scope or an enclosing block scope), so that its stack position falls below the `OpFreeStack` discard range.

If a block allocates a new text variable internally and returns it, the variable's position falls inside the discard range and the debug assertion fires. The fix in such cases is to hoist the text variable's initialisation (`claim_temp`) to the enclosing scope.

### `copy_variable` (`variables/`)

Creates an exact duplicate of a variable (same name, same type including deps) with fresh `scope = u16::MAX` and `stack_pos = u16::MAX`. Used when a variable from an outer scope is assigned again in an inner sibling scope that no longer has the outer scope in its stack.

---

## Rust code generation (`src/generation/`)

`src/generation/` provides the `Output` struct and `rust_type` function used to transpile compiled loft programs to Rust source files. This is used only during development to regenerate `src/fill.rs` and `src/native.rs` from the `#rust "..."` annotations in the default library. It is not involved in the normal interpreter execution path.

### `Output<'a>`

```rust
pub struct Output<'a> {
    pub data: &'a Data,         // read-only view of all definitions
    pub stores: &'a Stores,     // runtime type schema
    pub counter: u32,           // unique label counter for generated identifiers
    pub def_nr: u32,            // definition number currently being emitted
    pub indent: u32,            // current indentation level
    pub declared: HashSet<u16>, // variable slots already declared in this function
}
```

Bundles the read-only compile-time data with the mutable emission state so that individual emit functions receive a single context argument.

### `rust_type(tp, context) -> String`

Maps a loft `Type` to the corresponding Rust type string. The `context` parameter controls the form:

| Context | Effect |
|---|---|
| `Context::Argument` | Stack/argument passing type (e.g. `Str` for text, `i32` for integer) |
| `Context::Variable` | Local variable type (e.g. `String` for text — owned heap allocation) |
| `Context::Reference` | Prefixes the argument type with `&` |

Integer types are mapped to `u8`/`u16`/`i8`/`i16`/`i32` based on the `Integer(min, max)` range. Reference, vector, and collection types all map to `DbRef`.

---

## Bytecode generation (`src/compile.rs`, `src/state/`)

`byte_code(state, data)` iterates all `Function` definitions (excluding operators) and calls `state.def_code(d_nr, data)` for each. This compiles the `Value` IR tree into a flat bytecode representation stored in `State`.

The bytecode is a compact encoding of the `Call`/`Set`/`If`/`Loop` IR nodes. It is optimised for fast interpretation rather than size.

`state.execute("main", data)` runs the named function.

`show_code(writer, state, data)` dumps both the IR tree and the bytecode for each user-defined function to a writer — used for the debug output in `tests/dumps/`.

---

## Default library (`default/*.loft`)

The default library is loaded before any user source. It is parsed with `default: true`, which:
- Allows `OpXxx`-prefixed names (operator definitions).
- Allows `#rust "..."` annotations that supply the Rust implementation string for the code generator (`src/generation/`).
- Registers all built-in types, operators, and standard functions in `Data` and `Stores`.

Files are loaded in alphabetical order:
- `01_code.loft` — all operators and standard functions
- `02_images.loft` — image, file, pixel types
- `03_text.loft` — text utility functions

---

## Naming conventions enforced by the parser

| Category | Convention | Enforcement |
|---|---|---|
| Functions / variables | `lower_case` | `is_lower()` |
| Types / structs / enums / enum values | `CamelCase` | `is_camel()` |
| Constants | `UPPER_CASE` | (noted but not enforced by `is_upper`) |
| Operator definitions | `OpXxx` prefix | `is_op()` |

Violations emit an `Error` diagnostic but do not abort compilation.

---

## Diagnostic system (`src/diagnostics.rs`)

All errors, warnings, and fatal messages flow through `Diagnostics`:

```
Warning  — informational; compilation continues
Error    — type/syntax error; second pass is skipped if errors found in first pass
Fatal    — parse cannot continue (e.g. unterminated string, syntax error)
```

Diagnostics are collected on the `Lexer` and merged into `Parser::diagnostics` after each parse call. The `diagnostic!` and `specific!` macros format messages with file/line/column from `self.lexer.pos()`.

---

## Source file summary

| File | Role |
|---|---|
| `src/lexer.rs` | Tokeniser; link/revert backtracking; string/format mode |
| `src/parser/mod.rs` | `Parser` struct, constructors, `parse`/`parse_dir`/`parse_file`, core helpers |
| `src/parser/definitions.rs` | Enum/struct/typedef/function parsing; `enum_fn` dispatch synthesis |
| `src/parser/expressions.rs` | Expression parsing: operators, assignments, strings, function references |
| `src/parser/collections.rs` | Iterators, `for` loops, `map`/`filter`, parallel-for, vector comprehensions |
| `src/parser/control.rs` | Control flow: `if`, `while`, `return`, `parse_call`, `parse_method` |
| `src/parser/builtins.rs` | Parallel worker parsing helpers |
| `src/data.rs` | `Value`, `Type`, `DefType`, `Data`, `Attribute` definitions |
| `src/typedef.rs` | Type resolution; `Stores` schema population |
| `src/scopes.rs` | Scope assignment; lifetime cleanup insertion |
| `src/variables/` | Per-function variable table (`Function`) |
| `src/compile.rs` | `byte_code` — IR → bytecode; `show_code` |
| `src/state/mod.rs` | `State` struct, constructors, `execute`/`execute_argv`, stack primitives |
| `src/state/text.rs` | String/text operations: allocation, formatting, slicing |
| `src/state/io.rs` | File I/O, database manipulation, vector/hash/record operations |
| `src/state/codegen.rs` | Bytecode generation: `generate`, `generate_set`, all `gen_*` helpers |
| `src/state/debug.rs` | Debug dump: `dump_code`, `dump_op_arg`, `print_code`, log step tracing |
| `src/diagnostics.rs` | Error/warning collection and formatting |
| `src/database/mod.rs` | `Stores` constructor, basic get/put, parse-key helpers |
| `src/database/types.rs` | Type-building methods: `structure`, `field`, `finish`, `sorted`, `hash`, etc. |
| `src/database/allocation.rs` | Store management, claim/free, `copy_claims*`, `clone_for_worker` |
| `src/database/search.rs` | Find/iterate: `find`, `find_vector`, `find_array`, `find_index`, `next` |
| `src/database/structures.rs` | Record construction, parsing, `get_ref`, `get_field`, `vector_add` |
| `src/database/io.rs` | File I/O: `read_data`, `write_data`, `get_file`, `get_dir`, `get_png` |
| `src/database/format.rs` | Display/formatting: `show`, `dump`, `rec`, `path` |
| `src/generation/` | Rust code generator — `Output` struct, `rust_type` mapping, emits `fill.rs` / `native.rs` |
| `src/calc.rs` | Field byte-offset calculator for struct/enum-variant layout |
| `src/stack.rs` | Bytecode-generation stack frame (`Stack`, `Loop`) |
| `src/create.rs` | Drives code generation: `generate_lib` and `generate_code` |
| `default/*.loft` | Built-in operators and standard library |

---

## See also
- [INTERMEDIATE.md](INTERMEDIATE.md) — Value/Type enums in detail; 233 bytecode operators; State layout
- [INTERNALS.md](INTERNALS.md) — calc.rs, stack.rs, create.rs, native.rs, ops.rs, parallel.rs
- [TESTING.md](TESTING.md) — Test framework, LogConfig debug-logging presets
- [../DEVELOPERS.md](../DEVELOPERS.md) — How to add features: pipeline walkthrough, caveats per subsystem, debugging strategy
