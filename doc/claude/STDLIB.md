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
Both `single` and `float` variants exist for every function — choose `single` for speed, `float` for precision.

### Constants

| Name | Value | Description |
|------|-------|-------------|
| `PI` | 3.14159… | The ratio of a circle's circumference to its diameter. |
| `E`  | 2.71828… | Euler's number, the base of natural logarithms. |

### General

| Function | Description |
|----------|-------------|
| `abs(v: integer) -> integer` | Absolute value. Removes the sign from a negative integer. |
| `abs(v: long) -> long` | Absolute value for long integers. |
| `abs(v: single) -> single` | Absolute value for single-precision floats. |
| `abs(v: float) -> float` | Absolute value for double-precision floats. |
| `min(a: integer, b: integer) -> integer` | Smallest of two values. Returns null if either is null. |
| `min(a: long, b: long) -> long` | Long variant. |
| `min(a: single, b: single) -> single` | Single-precision variant. |
| `min(a: float, b: float) -> float` | Double-precision variant. |
| `max(a: integer, b: integer) -> integer` | Largest of two values. Returns null if either is null. |
| `max(a: long, b: long) -> long` | Long variant. |
| `max(a: single, b: single) -> single` | Single-precision variant. |
| `max(a: float, b: float) -> float` | Double-precision variant. |
| `clamp(v: integer, lo: integer, hi: integer) -> integer` | Clamps `v` to `[lo, hi]`. Returns null if any argument is null. |
| `clamp(v: long, lo: long, hi: long) -> long` | Long variant. |
| `clamp(v: single, lo: single, hi: single) -> single` | Single-precision variant. |
| `clamp(v: float, lo: float, hi: float) -> float` | Double-precision variant. |

### Rounding

| Function | Description |
|----------|-------------|
| `floor(v: single) -> single` | Round down to the nearest integer value. |
| `floor(v: float) -> float` | Double-precision floor. |
| `ceil(v: single) -> single` | Round up to the nearest integer value. |
| `ceil(v: float) -> float` | Double-precision ceil. |
| `round(v: single) -> single` | Round to the nearest integer value (half rounds away from zero). |
| `round(v: float) -> float` | Double-precision round. |
| `sqrt(v: single) -> single` | Square root. |
| `sqrt(v: float) -> float` | Double-precision square root. |

### Power and Logarithm

| Function | Description |
|----------|-------------|
| `pow(base: single, exp: single) -> single` | Raises `base` to the power `exp`. |
| `pow(base: float, exp: float) -> float` | Double-precision power. |
| `log(v: single, base: single) -> single` | Logarithm of `v` in the given `base`. |
| `log(v: float, base: float) -> float` | Double-precision logarithm. |
| `exp(v: single) -> single` | Raises E to the power `v` (single-precision). |
| `exp(v: float) -> float` | Double-precision natural exponential. |
| `ln(v: single) -> single` | Natural logarithm (single-precision). |
| `ln(v: float) -> float` | Double-precision natural logarithm. |
| `log2(v: single) -> single` | Base-2 logarithm (single-precision). |
| `log2(v: float) -> float` | Double-precision base-2 logarithm. |
| `log10(v: single) -> single` | Base-10 logarithm (single-precision). |
| `log10(v: float) -> float` | Double-precision base-10 logarithm. |

### Trigonometry

| Function | Description |
|----------|-------------|
| `cos(angle: single) -> single` | Cosine. |
| `cos(angle: float) -> float` | Double-precision cosine. |
| `sin(angle: single) -> single` | Sine. |
| `sin(angle: float) -> float` | Double-precision sine. |
| `tan(angle: single) -> single` | Tangent. |
| `tan(angle: float) -> float` | Double-precision tangent. |
| `acos(v: single) -> single` | Arc cosine. Returns angle (radians) whose cosine is `v`. |
| `acos(v: float) -> float` | Double-precision arc cosine. |
| `asin(v: single) -> single` | Arc sine. Returns angle whose sine is `v`. |
| `asin(v: float) -> float` | Double-precision arc sine. |
| `atan(v: single) -> single` | Arc tangent. Returns angle in (-PI/2, PI/2). |
| `atan(v: float) -> float` | Double-precision arc tangent. |
| `atan2(y: single, x: single) -> single` | Arc tangent of `y/x`, preserving the correct quadrant. |
| `atan2(y: float, x: float) -> float` | Double-precision atan2. |

---

## Text

Functions for working with `text` (UTF-8 strings) and `character` values.

### Length

| Function | Description |
|----------|-------------|
| `len(v: text) -> integer` | Number of bytes in the text. |
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

All three require a compile-time function reference (`fn <name>`). The function reference must be a named top-level function — lambda expressions are not yet supported (T2-1).

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

Worker must return a primitive: `integer`, `long`, `float`, or `boolean`. Input must be a `vector<T>`.

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
