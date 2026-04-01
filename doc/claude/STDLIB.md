# Loft Standard Library Reference

This document describes all public functions, constants, and types available in the loft standard library.

## Contents
- [Implementation notes](#implementation-notes)
- [Types](#types)
- [Math](#math)
- [Text](#text)
- [Collections](#collections)
- [Keyed collections (hash / index / sorted)](#keyed-collections-hash--index--sorted)
- [Output and Diagnostics](#output-and-diagnostics)
- [Logging](#logging)
- [File System](#file-system)
- [Parallel](#parallel)
- [Environment](#environment)
- [Random](#random)

---

## Implementation notes

Standard library functions fall into two implementation categories:

- **Loft-implemented** — defined in `default/01_code.loft`, `default/02_images.loft`, or `default/03_text.loft` using the loft language itself. These have a normal function body.
- **Native (Rust)** — declared in the default library with a `#rust "..."` annotation and implemented as hand-written Rust functions in `src/native.rs`. These handle OS interaction and operations that cannot be expressed in loft (file I/O, environment variables, string classification, etc.).

See [INTERNALS.md](INTERNALS.md) for the full list of native functions, their Rust names, and the naming convention (`n_<func>` for globals, `t_<N><Type>_<method>` for methods).

---

## Types

The primitive types built into loft.

| Type        | Size   | Description |
|-------------|--------|-------------|
| `boolean`   | 1 byte | True or false value. |
| `integer`   | 4 bytes | 32-bit signed integer. |
| `long`      | 8 bytes | 64-bit signed integer. Use when values exceed ~2 billion. |
| `single`    | 4 bytes | 32-bit floating-point. Good for graphics and performance-sensitive math. |
| `float`     | 8 bytes | 64-bit floating-point. Use when precision matters. |
| `text`      | —      | UTF-8 string. |
| `character` | 4 bytes | A single Unicode code point. |

**Integer subtypes** (ranged aliases for compact storage):

| Type  | Range           | Size   |
|-------|-----------------|--------|
| `u8`  | 0 – 255         | 1 byte |
| `i8`  | -128 – 127      | 1 byte |
| `u16` | 0 – 65535       | 2 bytes |
| `i16` | -32768 – 32767  | 2 bytes |
| `i32` | full integer    | 4 bytes |

Use the sized subtypes in struct fields to reduce memory usage. They behave as `integer` in expressions.

---

## Math

Functions for numeric computation. All trigonometric functions work in radians.

In the tables below, **N** = `integer | long | single | float` for general functions, and **F** = `single | float` for float-only functions. Use `single` for speed, `float` for precision.

### Constants

| Name | Value | Description |
|------|-------|-------------|
| `PI` | 3.14159… | Ratio of a circle's circumference to its diameter. |
| `E`  | 2.71828… | Euler's number, base of natural logarithms. |

### General (N = integer | long | single | float)

| Function | Description |
|----------|-------------|
| `abs(v: N) -> N` | Absolute value. |
| `min(a: N, b: N) -> N` | Smaller of two values. Returns null if either is null. |
| `max(a: N, b: N) -> N` | Larger of two values. Returns null if either is null. |
| `clamp(v: N, lo: N, hi: N) -> N` | Clamps `v` to `[lo, hi]`. Returns null if any arg is null. |

### Rounding and roots (F = single | float)

| Function | Description |
|----------|-------------|
| `floor(v: F) -> F` | Round down to nearest integer value. |
| `ceil(v: F) -> F` | Round up to nearest integer value. |
| `round(v: F) -> F` | Round to nearest (half rounds away from zero). |
| `sqrt(v: F) -> F` | Square root. |

### Power and Logarithm (F = single | float)

| Function | Description |
|----------|-------------|
| `pow(base: F, exp: F) -> F` | Raises `base` to the power `exp`. |
| `exp(v: F) -> F` | Raises E to the power `v`. |
| `ln(v: F) -> F` | Natural logarithm. |
| `log(v: F, base: F) -> F` | Logarithm in the given `base`. |
| `log2(v: F) -> F` | Base-2 logarithm. |
| `log10(v: F) -> F` | Base-10 logarithm. |

### Trigonometry (F = single | float, angles in radians)

| Function | Description |
|----------|-------------|
| `cos(angle: F) -> F` | Cosine. |
| `sin(angle: F) -> F` | Sine. |
| `tan(angle: F) -> F` | Tangent. |
| `acos(v: F) -> F` | Arc cosine — returns angle whose cosine is `v`. |
| `asin(v: F) -> F` | Arc sine — returns angle whose sine is `v`. |
| `atan(v: F) -> F` | Arc tangent — returns angle in (-PI/2, PI/2). |
| `atan2(y: F, x: F) -> F` | Arc tangent of `y/x`, preserving quadrant. |

---

## Text

Functions for working with `text` (UTF-8 strings) and `character` values.

### Length

| Function | Description |
|----------|-------------|
| `len(v: text) -> integer` | Number of bytes in the text. |
| `size(v: text) -> integer` | Number of Unicode code points (characters) in the text. |
| `len(v: character) -> integer` | Byte length of the character's UTF-8 encoding (1–4). |

### Searching

| Function | Description |
|----------|-------------|
| `find(self: text, value: text) -> integer` | Returns the byte index of the first occurrence of `value`, or null if not found. |
| `rfind(self: text, value: text) -> integer` | Returns the byte index of the last occurrence of `value`, or null if not found. |
| `contains(self: text, value: text) -> boolean` | Returns true if `value` appears anywhere in `self`. |
| `starts_with(self: text, value: text) -> boolean` | Returns true if `self` begins with `value`. |
| `ends_with(self: text, value: text) -> boolean` | Returns true if `self` ends with `value`. |

### Transformation

| Function | Description |
|----------|-------------|
| `replace(self: text, value: text, with: text) -> text` | Returns a copy of `self` with every occurrence of `value` replaced by `with`. |
| `to_lowercase(self: text) -> text` | Returns a lowercase copy. |
| `to_uppercase(self: text) -> text` | Returns an uppercase copy. |
| `trim(self: text) -> text` | Removes leading and trailing whitespace. Use when processing user input or file content. |
| `trim_start(self: text) -> text` | Removes leading whitespace only. |
| `trim_end(self: text) -> text` | Removes trailing whitespace only. |
| `split(self: text, separator: character) -> vector<text>` | Splits `self` on every occurrence of `separator` and returns the parts as a vector. |

### Iterating over text

`for c in some_text` yields one `character` per UTF-8 code point.

Inside the loop body two positional attributes are available:

| Attribute | Type      | Meaning                                                          |
|-----------|-----------|------------------------------------------------------------------|
| `c#index` | `integer` | Byte offset of the **start** of the current character in the string. |
| `c#next`  | `integer` | Byte offset immediately **after** the current character (= start of next char). |

These satisfy: `c#next == c#index + len(c)`.

Example — split on a separator character without using `split()`:
```
parts = [];
p = 0;
for c in path {
    if c == '/' {
        parts += [path[p..c#index]];
        p = c#next;
    }
}
```

### Character Classification

These functions return true only if **every character** in the text satisfies the condition.
The single-`character` variants test one code point.

| Function | Description |
|----------|-------------|
| `is_lowercase(self: text/character) -> boolean` | All characters are lowercase letters. |
| `is_uppercase(self: text/character) -> boolean` | All characters are uppercase letters. |
| `is_numeric(self: text/character) -> boolean` | All characters are numeric digits (Unicode numeric, not just ASCII 0–9). |
| `is_alphanumeric(self: text/character) -> boolean` | All characters are letters or digits. |
| `is_alphabetic(self: text/character) -> boolean` | All characters are alphabetic. |
| `is_whitespace(self: text) -> boolean` | All characters are whitespace. |
| `is_control(self: text) -> boolean` | All characters are control characters. |

### Joining

| Function | Description |
|----------|-------------|
| `join(parts: vector<text>, sep: text) -> text` | Joins the elements of `parts` with `sep` between each consecutive pair. Returns `""` for an empty vector. Use to build comma-separated lists, path segments, or any delimited output. |

---

## Collections

Operations on `vector<T>` — the primary ordered collection type.

| Function | Description |
|----------|-------------|
| `len(v: vector) -> integer` | Number of elements in the vector. Use in loop bounds: `for i in 0..v.len()`. |

### Aggregates

| Function | Description |
|----------|-------------|
| `sum_of(v: vector<integer>) -> integer` | Sum of all elements; returns 0 for an empty vector. |
| `min_of(v: vector<integer>) -> integer` | Smallest element. |
| `max_of(v: vector<integer>) -> integer` | Largest element. |

Vectors are grown by appending with `+=` and elements are accessed by index. Removal and insertion are handled by the parser's built-in operators.

| Operation | Description |
|-----------|-------------|
| `v += [elem]` | Append one element. |
| `v.remove(i)` | Remove element at index `i` (negative counts from end); returns `boolean`. |
| `v#remove` | Remove current element inside a `for ... if ...` loop. |

---

## Keyed collections (hash / index / sorted)

All three keyed collection types share a common lookup and removal syntax handled by the parser, not the stdlib:

| Syntax | Description |
|--------|-------------|
| `c[key]` | Look up element by key; returns the element or `null` if absent. |
| `c[key] = null` | Remove the element with that key; no-op if absent. |
| `e#remove` | Remove current element during a `for ... if ...` iteration. |

These are parser-level operations; they compile to `OpGetRecord`, `OpHashRemove`, and `OpRemove` respectively. There are no corresponding callable functions.

---

## Output and Diagnostics

| Function | Description |
|----------|-------------|
| `print(v: text)` | Writes `v` to standard output without a newline. |
| `println(v: text)` | Writes `v` followed by a newline. |
| `assert(test: boolean, message: text)` | Panics with `message` if `test` is false. In production mode (`--production` CLI flag), writes an `error` log entry instead of aborting. |
| `panic(message: text)` | Immediately terminates execution with `message`. In production mode, writes a `fatal` log entry instead of aborting. |

---

## Logging

Structured file-based output from running loft programs. Logging is configured via `log.conf` beside the main `.loft` file (or `--log-conf <path>`). See [LOGGER.md](LOGGER.md) for full configuration reference.

| Function | Description |
|----------|-------------|
| `log_info(message: text)` | Writes a record at `INFO` severity. Silently discarded if no logger or below the configured level. |
| `log_warn(message: text)` | Writes a record at `WARN` severity (default minimum level). |
| `log_error(message: text)` | Writes a record at `ERROR` severity. |
| `log_fatal(message: text)` | Writes a record at `FATAL` severity. Does **not** abort (use `panic()` to abort). |

The loft source file and line number are injected by the compiler at each call site — the log record always shows exactly where in the loft code the log call was made.

Rate limiting: at most 5 messages per 60-second window per call site (configurable). Suppressed messages are counted and a notice is emitted when the window resets.

```
2026-03-13T14:05:32.417Z WARN  src/compute.loft:142  division result may overflow
```

---

## File System

Types and functions for reading and writing files. A `File` value is obtained via `file()` and carries the path, format, and an internal reference.

### Types

**`Format`** (enum): Describes how a file is opened.

| Value           | Description |
|-----------------|-------------|
| `Format.TextFile`     | Default. Read or write as UTF-8 text. |
| `Format.LittleEndian` | Binary mode, least-significant byte first. |
| `Format.BigEndian`    | Binary mode, most-significant byte first. |
| `Format.Directory`    | Represents a directory path. |

**`File`**: A handle to a filesystem entry. Fields: `path: text`, `size: long`, `format: Format`.

### Opening Files

| Function | Description |
|----------|-------------|
| `file(path: text) -> File` | Opens the file at `path` and returns a `File` handle. |

### Reading Text Files

| Function | Description |
|----------|-------------|
| `content(self: File) -> text` | Reads the entire file as a UTF-8 text value. |
| `lines(self: File) -> vector<text>` | Reads the file and splits it into lines. |

### Writing Text Files

| Function | Description |
|----------|-------------|
| `write(self: File, v: text)` | Writes `v` as UTF-8 text to the file. Overwrites existing content. |

### Binary Files

Binary mode must be activated before reading or writing raw data. Use `f.format = Format.LittleEndian` or `f.format = Format.BigEndian` to enable binary mode.

| Function | Description |
|----------|-------------|
| `little_endian(self: File)` | Switches the file to little-endian binary mode. |
| `big_endian(self: File)` | Switches the file to big-endian binary mode. |
| `write_bin(self: File, v: reference)` | Writes a struct value as raw binary data. File must be in binary mode first. |
| `read(self: File, v: reference)` | Reads binary data into a struct value. File must be in binary mode first. |
| `seek(self: File, pos: long)` | Moves the read/write position to `pos` bytes from the start. |

**Binary attribute operators on `f: File`:**

| Syntax | Description |
|--------|-------------|
| `f += integer` | Writes 4 bytes (integer) in the current endian format. |
| `f += long` | Writes 8 bytes (long) in the current endian format. |
| `f += single` | Writes 4 bytes (single) in the current endian format. |
| `f#read(n) as T` | Reads `n` bytes and interprets as type `T` (e.g. `i32`, `u8`, `long`). Returns null if fewer than `n` bytes are available (for non-text types). |
| `f#size` | Returns the current file size in bytes as `long`. |
| `f#index` | Returns the byte offset where the last read started (the `current` field). |
| `f#next` | Returns the current byte position (after last read). |
| `f#next = pos` | Seeks the file to `pos` (long). Only works after the file has been opened by a prior read or write. |
| `f#exists` | Returns `true` if the file or directory exists (format ≠ `Format.NotExists`). |
| `f#format` | Reads the `Format` enum value of `f`. |
| `f#format = Format.X` | Sets the format of `f`. |

**Notes:**
- `f += "text"` writes raw UTF-8 bytes; supported for TextFile, LittleEndian, and BigEndian modes.
- For new files (format=NotExists), `f += value` defaults to TextFile mode and creates the file.
- `f#read(n) as text` reads exactly `n` bytes (or fewer at EOF) as a UTF-8 string.
- `f#next = pos` is a no-op if called before the first read or write (the OS file handle does not exist until first I/O). Always perform a read or write before seeking.

### Directories

| Function | Description |
|----------|-------------|
| `files(self: File) -> vector<File>` | Returns the entries inside a directory. The `File` must have `format == Format.Directory`. Use to iterate over all files in a folder. |

### Filesystem Operations

Mutating filesystem operations return a `FileResult` enum:

| Variant | Meaning |
|---------|---------|
| `FileResult.Ok` | Operation succeeded. |
| `FileResult.NotFound` | Path does not exist or is outside the project directory. |
| `FileResult.PermissionDenied` | OS permission denied. |
| `FileResult.IsDirectory` | Expected a file, got a directory. |
| `FileResult.NotDirectory` | Expected a directory, got a file. |
| `FileResult.Other` | Any other OS error. |

| Function | Description |
|----------|-------------|
| `ok(self: FileResult) -> boolean` | Returns `true` if `Ok`. |
| `exists(path: text) -> boolean` | Returns `true` if the path exists and is inside the project. |
| `delete(path: text) -> FileResult` | Removes a file. |
| `move(from: text, to: text) -> FileResult` | Renames or relocates a file within the project. |
| `mkdir(path: text) -> FileResult` | Creates a single directory level. |
| `mkdir_all(path: text) -> FileResult` | Creates a directory and all missing parents. |
| `set_file_size(self: File, size: long) -> FileResult` | Truncates or extends a file to exactly `size` bytes. |

### Images

| Function | Description |
|----------|-------------|
| `png(self: File) -> Image` | Decodes a PNG file and returns an `Image`. Returns null if the file is not in text format. |

**`Image`** struct fields: `name: text`, `width: integer`, `height: integer`, `data: vector<Pixel>`.

**`Pixel`** struct fields: `r: integer`, `g: integer`, `b: integer` (each 0–255).

| Function | Description |
|----------|-------------|
| `value(self: Pixel) -> integer` | Returns the pixel colour as a packed 24-bit integer (`0xRRGGBB`). Use for fast colour comparison or storage. |

---

## JSON / Parsing

| Expression | Description |
|---|---|
| `"{value:j}"` | Serialise any struct/enum/vector to JSON text |
| `Type.parse(text)` | Parse JSON or loft-native text into a struct |
| `vector<T>.parse(text)` | Parse a JSON array into an iterable vector |
| `record#errors` | Iterate parse errors from the last `Type.parse()` call |

```loft
user = User.parse(`{{"id":42,"name":"Alice"}}`);
scores = vector<Score>.parse(`[{{"value":10}},{{"value":20}}]`);
for e in user#errors { log_warn(e); }
```

---

## Higher-order functions

`map`, `filter`, and `reduce` are compiler special-cases (like `parallel_for`) — they take a `fn <name>` function reference and a vector and produce a new vector or scalar.

| Signature | Description |
|---|---|
| `map(v: vector<T>, f: fn(T) -> U) -> vector<U>` | Applies `f` to each element and collects the results |
| `filter(v: vector<T>, pred: fn(T) -> boolean) -> vector<T>` | Keeps only elements for which `pred` returns `true` |
| `reduce(v: vector<T>, init: U, f: fn(U, T) -> U) -> U` | Left-folds `v` starting from `init`, applying `f(acc, elm)` at each step |

```loft
fn double(x: integer) -> integer { x * 2 }
fn is_pos(x: integer) -> boolean { x > 0 }
fn add(a: integer, b: integer) -> integer { a + b }

doubled  = map(nums, fn double);         // [2, 4, 6, ...]
positive = filter(nums, fn is_pos);      // only positive elements
total    = reduce(nums, 0, fn add);      // sum of all elements
```

All three accept either a named function reference (`fn <name>`) or a lambda expression:

```loft
doubled = map(nums, fn(x: integer) -> integer { x * 2 });
evens   = filter(nums, fn(x: integer) -> boolean { x % 2 == 0 });
total   = reduce(nums, 0, fn(acc: integer, x: integer) -> integer { acc + x });
```

Lambdas that capture variables from the enclosing scope (closures) also work:

```loft
factor = 3;
scaled = map(nums, fn(x: integer) -> integer { x * factor });
```

Capture is by value at definition time — later changes to `factor` do not
affect the lambda.  See [LOFT.md § Closures](LOFT.md) for details.

---

## Parallel

The public parallel API is the `par(...)` for-loop clause. The internal functions `parallel_for` and `parallel_for_int` are not part of the user API.

Function references (`fn <name>`, type `fn(T) -> R`) are first-class callable values — they can be stored in variables, passed as parameters, and called directly (`f(args)`), not only as `par(...)` worker arguments. See [LOFT.md](LOFT.md) § Literals for the full syntax.

### `par(...)` Parallel For-Loop

```loft
for a in <vector> par(b=<worker_call>, <threads>) {
    // body — b holds the worker result for this element
}
```

Two worker call forms:

| Form | Example | Description |
|---|---|---|
| Form 1 | `func(a)` | Global function called with the loop element |
| Form 2 | `a.method()` | Method on the element type |

Supported return types: `integer`, `long`, `float`, `single`, `boolean`, inline `enum`, `text`.
Extra context arguments are forwarded: `par(b=scale(a, mult), N)`.
Input must be a `vector<T>`.

```loft
struct Score { value: integer }

fn double_score(r: const Score) -> integer { r.value * 2 }
fn get_value(self: const Score) -> integer { self.value }

fn main() {
    q = make_scores();   // vector of Score

    // Form 1: global function
    sum = 0;
    for a in q.items par(b=double_score(a), 4) {
        sum += b;
    }

    // Form 2: method
    total = 0;
    for a in q.items par(b=a.get_value(), 1) {
        total += b;
    }
}
```

**Worker function rules:**
- Accept a single `const` reference as the first parameter.
- Do not name the first parameter `self` (this makes it a method, looked up differently).
- Workers receive a read-only store snapshot; writing to input data panics.
- No nested parallelism.

**Limitations:**
- Float/long result accumulation in the loop body: if `b` is float/long, using it in arithmetic with a pre-declared float/long variable can trigger a first-pass type-inference conflict. Workaround: use `b` only in boolean comparisons or cast (`sum += b as integer`).
- Implementation: `src/parallel.rs`; see [THREADING.md](THREADING.md) for internals.

---

## Environment

Functions for interacting with the host operating system.

### Command-Line Arguments

| Function | Description |
|----------|-------------|
| `arguments() -> vector<text>` | Returns the command-line arguments passed to the program. The first element is typically the program name. |

### Environment Variables

| Function | Description |
|----------|-------------|
| `env_variable(name: text) -> text` | Returns the value of the environment variable `name`, or null if it is not set. |
| `env_variables() -> vector<EnvVariable>` | Returns all environment variables as a vector of `EnvVariable` records (fields: `name`, `value`). |

### Paths

| Function | Description |
|----------|-------------|
| `directory(v: &text = "") -> text` | Returns the current working directory, optionally with `v` appended as a subpath. Use to construct absolute paths relative to where the program was launched. |
| `user_directory(v: &text = "") -> text` | Returns the current user's home directory, optionally with `v` appended. |
| `program_directory(v: &text = "") -> text` | Returns the directory containing the running executable, optionally with `v` appended. |

---

## Random

A fast PCG64 generator, seeded with a fixed default at startup. Call `rand_seed` before use
when reproducibility matters.

| Function | Description |
|----------|-------------|
| `rand(lo: integer, hi: integer) -> integer` | Returns a uniformly distributed random integer in `[lo, hi]` (inclusive). Returns null if `lo > hi` or either bound is null. |
| `rand_seed(seed: long)` | Seeds the thread-local RNG. Same seed always produces the same sequence. |
| `rand_indices(n: integer) -> vector<integer>` | Returns a vector of `n` integers `[0, 1, ..., n-1]` in a random order. Empty when `n ≤ 0`. Useful for random iteration or sampling without replacement. |

**Example — pick 3 distinct items at random:**
```loft
rand_seed(42);
items = ["a", "b", "c", "d", "e"];
order = rand_indices(len(items));
for i in 0..3 { println(items[order[i]]) }
```

---

## See also
- [LOFT.md](LOFT.md) — Loft language reference (syntax, types, operators, control flow)
- [INTERNALS.md](INTERNALS.md) — Native function registry, `src/native.rs`, `src/ops.rs`
