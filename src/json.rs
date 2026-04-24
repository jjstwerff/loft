// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// JSON parser used by the `json_parse` native function in
// `src/native.rs`.  Walks UTF-8 text once and returns a `Parsed`
// value that the caller materialises into a loft `JsonValue`
// struct-enum record.
//
// Step 4 scope: full RFC 8259 — null, true, false, number, string
// (incl. standard escapes), array, object.  The `Parsed` tree is
// fully recursive here; `native::n_json_parse` flattens it into
// the arena-indexed loft JsonValue form at materialisation time.
//
// Q1 (this commit): parse failures carry a JSON Pointer path
// (RFC 6901) plus the byte offset.  Line:column and the
// surrounding context snippet are computed by `format_error`
// at error-formatting time, not per token, so the success path
// pays nothing.

/// Intermediate tree produced by [`parse`].  The loft-level
/// `JsonValue` variants are built from these values inside
/// `native::n_json_parse` so this module stays free of database
/// concerns.
///
/// The `Ident` variant is produced only by `Dialect::Lenient`
/// for a bare identifier in value position (e.g. `Daily` in
/// `{category: Daily}`, where Daily is a loft enum tag).  The
/// distinction is preserved so the walker can dispatch
/// strictly: text fields accept `Str` only, enum fields accept
/// either `Str` or `Ident`.  `Dialect::Strict` never emits
/// `Ident`.
#[derive(Debug, Clone)]
pub enum Parsed {
    Null,
    Bool(bool),
    Number(f64),
    Str(String),
    Ident(String),
    Array(Vec<Parsed>),
    /// Object entries carry the byte offset of the key within the
    /// original input — used by the schema walker to produce
    /// `"line N:M path:X"` diagnostics on shape mismatches without
    /// re-scanning the source.  Tuple shape: `(name, key_byte_offset, value)`.
    Object(Vec<(String, usize, Parsed)>),
}

/// Input dialect selector.
///
/// * `Strict` — RFC 8259 JSON.  Object keys must be quoted
///   strings, no extensions.  This is what `json_parse(text)`
///   uses and is the public surface for user-supplied JSON.
/// * `Lenient` — accepts the same grammar as `Strict` *plus*
///   loft's bare-identifier object keys (`{val: 7}`) that the
///   legacy `vector<T>.parse(text)` path has supported since
///   day one.  This keeps loft-authored data literals compiling
///   through the unified parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dialect {
    #[default]
    Strict,
    Lenient,
}

/// Structured parse error.  `path` is an RFC 6901 JSON Pointer
/// to the location in the input where parsing gave up — `""`
/// means "at the root", `/users/3/age` means "third element of
/// the `users` array's `age` field".  `byte_offset` is the
/// absolute byte position; line:column + context snippet are
/// derived by [`format_error`] at error-formatting time so the
/// success path pays nothing.
#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    pub byte_offset: usize,
    pub path: String,
}

/// Internal: a single parse step's result — value + advance, or
/// (message, offset).  The path is threaded out-of-band via the
/// `&mut Vec<String>` path stack so most arms don't need to mention it.
type ParseResult = Result<(Parsed, usize), (String, usize)>;

/// Parse the entire `input` as a JSON value in strict RFC 8259
/// mode.  Equivalent to `parse_with(input, Dialect::Strict)`.
///
/// Leading and trailing whitespace is allowed.  Characters after
/// the value (other than whitespace) are a syntax error — strict
/// RFC 8259, not a forgiving tokeniser.
///
/// # Errors
/// Returns a [`ParseError`] when the input is not valid JSON.
/// The `path` field localises the failure inside the document
/// (RFC 6901 JSON Pointer); the `byte_offset` field locates it
/// inside the raw text.
pub fn parse(input: &str) -> Result<Parsed, ParseError> {
    parse_with(input, Dialect::Strict)
}

/// Parse the entire `input` as a JSON value using the given
/// [`Dialect`].  See [`Dialect`] for the differences between
/// `Strict` (RFC 8259) and `Lenient` (loft data literals).
///
/// # Errors
/// Returns a [`ParseError`] when the input is not valid in the
/// chosen dialect.
pub fn parse_with(input: &str, dialect: Dialect) -> Result<Parsed, ParseError> {
    let bytes = input.as_bytes();
    let start = skip_ws(bytes, 0);
    let mut path: Vec<String> = Vec::new();
    let res = parse_value(bytes, start, &mut path, dialect);
    let (value, mut i) = match res {
        Ok(ok) => ok,
        Err((msg, at)) => {
            return Err(ParseError {
                message: msg,
                byte_offset: at,
                path: render_path(&path),
            });
        }
    };
    i = skip_ws(bytes, i);
    if i != bytes.len() {
        return Err(ParseError {
            message: format!("unexpected trailing byte at offset {i}"),
            byte_offset: i,
            path: render_path(&path),
        });
    }
    Ok(value)
}

/// Render the path stack as an RFC 6901 JSON Pointer.  Empty
/// stack → `""` (root).  Each segment is escaped: `~` → `~0`,
/// `/` → `~1`.
fn render_path(stack: &[String]) -> String {
    if stack.is_empty() {
        return String::new();
    }
    let mut out = String::with_capacity(stack.iter().map(|s| s.len() + 1).sum());
    for seg in stack {
        out.push('/');
        for ch in seg.chars() {
            match ch {
                '~' => out.push_str("~0"),
                '/' => out.push_str("~1"),
                _ => out.push(ch),
            }
        }
    }
    out
}

/// Convert a byte offset into 1-based (line, column).  Line
/// counts `\n`; column counts bytes since the last newline + 1.
/// Out-of-range offsets clamp to the input length.
#[must_use]
pub fn line_col_of(input: &str, byte_offset: usize) -> (usize, usize) {
    let bytes = input.as_bytes();
    let cap = byte_offset.min(bytes.len());
    let mut line = 1usize;
    let mut col_start = 0usize;
    for (i, b) in bytes[..cap].iter().enumerate() {
        if *b == b'\n' {
            line += 1;
            col_start = i + 1;
        }
    }
    (line, cap - col_start + 1)
}

/// Format a [`ParseError`] into a human-readable diagnostic with
/// path, line:column, message, and a context snippet (N lines
/// before the error, the error line with a caret, M lines after).
#[must_use]
pub fn format_error(input: &str, err: &ParseError, before: usize, after: usize) -> String {
    let (line, col) = line_col_of(input, err.byte_offset);
    let path_disp = if err.path.is_empty() {
        "(root)"
    } else {
        err.path.as_str()
    };
    let snippet = context_snippet(input, line, col, before, after);
    format!(
        "parse error at line {line} col {col} (byte {byte}):\n  path: {path_disp}\n  {msg}\n{snippet}",
        byte = err.byte_offset,
        msg = err.message,
    )
}

/// Build a `before`/error/`after` line snippet around `(line,
/// col)`.  The error line is followed by a caret `^` placed
/// under `col` (1-based).
fn context_snippet(input: &str, line: usize, col: usize, before: usize, after: usize) -> String {
    let lines: Vec<&str> = input.lines().collect();
    let lo = line.saturating_sub(before + 1);
    let hi = (line + after).min(lines.len());
    let width = hi.to_string().len();
    let mut out = String::new();
    use std::fmt::Write;
    for (idx, content) in lines.iter().enumerate().take(hi).skip(lo) {
        let n = idx + 1;
        let _ = writeln!(out, "    {n:>width$} \u{2502} {content}");
        if n == line {
            // caret line — width-wide gutter, vertical-bar, then
            // (col-1) spaces, then ^
            let spaces = " ".repeat(col.saturating_sub(1));
            let _ = writeln!(out, "    {pad:>width$} \u{2502} {spaces}^", pad = "");
        }
    }
    out
}

fn parse_value(bytes: &[u8], i: usize, path: &mut Vec<String>, dialect: Dialect) -> ParseResult {
    if i >= bytes.len() {
        return Err(("unexpected end of input".to_string(), i));
    }
    match bytes[i] {
        b'"' => parse_string(bytes, i),
        b'-' | b'0'..=b'9' => parse_number(bytes, i),
        b'[' => parse_array(bytes, i, path, dialect),
        b'{' => parse_object(bytes, i, path, dialect),
        c if dialect == Dialect::Lenient && (c.is_ascii_alphabetic() || c == b'_') => {
            let (tag, j) = parse_bare_identifier_value(bytes, i);
            // Struct-enum-variant-with-payload shape: `Tag { fields }`.
            // Represented as `Object([(tag_name, ident_start, Object(fields))])`
            // — a single-entry object whose key is the variant tag.  The
            // schema walker's Parts::Enum arm detects this shape and
            // dispatches to the variant's EnumValue struct.  Only applies
            // when the identifier is NOT a reserved word (null/true/false).
            if let Parsed::Ident(name) = &tag {
                let k = skip_ws(bytes, j);
                if k < bytes.len() && bytes[k] == b'{' {
                    let (obj, end) = parse_object(bytes, k, path, dialect)?;
                    return Ok((Parsed::Object(vec![(name.clone(), i, obj)]), end));
                }
            }
            Ok((tag, j))
        }
        b'n' => parse_literal(bytes, i, b"null", Parsed::Null),
        b't' => parse_literal(bytes, i, b"true", Parsed::Bool(true)),
        b'f' => parse_literal(bytes, i, b"false", Parsed::Bool(false)),
        b => Err((format!("unexpected byte {b:#x} at offset {i}"), i)),
    }
}

/// Parse a bare identifier in value position under
/// `Dialect::Lenient`.  Consumes `[A-Za-z_][A-Za-z0-9_]*`.
/// Reserved words `null` / `true` / `false` produce the
/// corresponding [`Parsed`] variant so callers don't have to
/// special-case them; any other identifier becomes
/// [`Parsed::Ident`].  Infallible because the caller only
/// invokes it after verifying the leading byte is alphabetic
/// or underscore.
fn parse_bare_identifier_value(bytes: &[u8], i: usize) -> (Parsed, usize) {
    let start = i;
    let mut j = i + 1;
    while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
        j += 1;
    }
    // Safe: all bytes accepted above are ASCII.
    let name = std::str::from_utf8(&bytes[start..j]).expect("ASCII identifier slice");
    let value = match name {
        "null" => Parsed::Null,
        "true" => Parsed::Bool(true),
        "false" => Parsed::Bool(false),
        _ => Parsed::Ident(name.to_string()),
    };
    (value, j)
}

fn parse_literal(bytes: &[u8], i: usize, word: &[u8], value: Parsed) -> ParseResult {
    if i + word.len() > bytes.len() || &bytes[i..i + word.len()] != word {
        return Err((
            format!(
                "expected `{}` at offset {i}",
                std::str::from_utf8(word).unwrap_or("?")
            ),
            i,
        ));
    }
    Ok((value, i + word.len()))
}

fn parse_string(bytes: &[u8], start: usize) -> ParseResult {
    debug_assert_eq!(bytes[start], b'"');
    let mut i = start + 1;
    let mut out = String::new();
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'"' => return Ok((Parsed::Str(out), i + 1)),
            b'\\' => {
                if i + 1 >= bytes.len() {
                    return Err(("unterminated escape".to_string(), i));
                }
                i += 1;
                match bytes[i] {
                    b'"' => out.push('"'),
                    b'\\' => out.push('\\'),
                    b'/' => out.push('/'),
                    b'b' => out.push('\u{0008}'),
                    b'f' => out.push('\u{000c}'),
                    b'n' => out.push('\n'),
                    b'r' => out.push('\r'),
                    b't' => out.push('\t'),
                    b'u' => {
                        if i + 4 >= bytes.len() {
                            return Err(("truncated \\uXXXX escape".to_string(), i));
                        }
                        let hex = std::str::from_utf8(&bytes[i + 1..i + 5])
                            .map_err(|_| ("non-ASCII in \\uXXXX escape".to_string(), i))?;
                        let cp = u32::from_str_radix(hex, 16)
                            .map_err(|_| ("invalid hex in \\uXXXX escape".to_string(), i))?;
                        if let Some(c) = char::from_u32(cp) {
                            out.push(c);
                        } else {
                            out.push('\u{fffd}');
                        }
                        i += 4;
                    }
                    other => return Err((format!("invalid escape \\{}", other as char), i)),
                }
                i += 1;
            }
            c if c < 0x20 => {
                return Err((format!("raw control byte {c:#x} in string"), i));
            }
            _ => {
                // UTF-8 continuation bytes are passed through verbatim
                // by pushing them into the String via bytes-to-str
                // reconstruction.  We know the input is `&str`, so the
                // slice is valid UTF-8.
                out.push(bytes[i] as char);
                i += 1;
            }
        }
    }
    Err(("unterminated string".to_string(), start))
}

fn parse_number(bytes: &[u8], start: usize) -> ParseResult {
    let mut i = start;
    if bytes[i] == b'-' {
        i += 1;
    }
    // integer part
    if i >= bytes.len() || !bytes[i].is_ascii_digit() {
        return Err(("expected digit in number".to_string(), i));
    }
    if bytes[i] == b'0' {
        i += 1;
    } else {
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
    // fraction
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        if i >= bytes.len() || !bytes[i].is_ascii_digit() {
            return Err(("expected digit after `.`".to_string(), i));
        }
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
    // exponent
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        i += 1;
        if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
            i += 1;
        }
        if i >= bytes.len() || !bytes[i].is_ascii_digit() {
            return Err(("expected digit in exponent".to_string(), i));
        }
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
    }
    let slice = std::str::from_utf8(&bytes[start..i])
        .map_err(|_| ("non-ASCII in number".to_string(), start))?;
    let n: f64 = slice
        .parse()
        .map_err(|_| (format!("invalid number `{slice}`"), start))?;
    Ok((Parsed::Number(n), i))
}

fn parse_array(
    bytes: &[u8],
    start: usize,
    path: &mut Vec<String>,
    dialect: Dialect,
) -> ParseResult {
    debug_assert_eq!(bytes[start], b'[');
    let mut i = skip_ws(bytes, start + 1);
    let mut items: Vec<Parsed> = Vec::new();
    if i < bytes.len() && bytes[i] == b']' {
        return Ok((Parsed::Array(items), i + 1));
    }
    let mut idx: usize = 0;
    loop {
        path.push(idx.to_string());
        let res = parse_value(bytes, i, path, dialect);
        let (v, j) = match res {
            Ok(ok) => ok,
            Err(e) => {
                // Leave path in place — render_path captures it
                // for the diagnostic.
                return Err(e);
            }
        };
        path.pop();
        items.push(v);
        i = skip_ws(bytes, j);
        if i >= bytes.len() {
            return Err(("unterminated array".to_string(), start));
        }
        match bytes[i] {
            b',' => {
                i = skip_ws(bytes, i + 1);
                idx += 1;
            }
            b']' => return Ok((Parsed::Array(items), i + 1)),
            b => return Err((format!("expected `,` or `]` in array, got {b:#x}"), i)),
        }
    }
}

#[allow(clippy::many_single_char_names)]
fn parse_object(
    bytes: &[u8],
    start: usize,
    path: &mut Vec<String>,
    dialect: Dialect,
) -> ParseResult {
    debug_assert_eq!(bytes[start], b'{');
    let mut i = skip_ws(bytes, start + 1);
    let mut fields: Vec<(String, usize, Parsed)> = Vec::new();
    if i < bytes.len() && bytes[i] == b'}' {
        return Ok((Parsed::Object(fields), i + 1));
    }
    loop {
        if i >= bytes.len() {
            return Err(("expected object key".to_string(), i));
        }
        let key_at = i;
        let (name, j) = parse_object_key(bytes, i, dialect)?;
        i = skip_ws(bytes, j);
        if i >= bytes.len() || bytes[i] != b':' {
            return Err(("expected `:` after object key".to_string(), i));
        }
        i = skip_ws(bytes, i + 1);
        path.push(name.clone());
        let res = parse_value(bytes, i, path, dialect);
        let (v, k) = res?;
        path.pop();
        fields.push((name, key_at, v));
        i = skip_ws(bytes, k);
        if i >= bytes.len() {
            return Err(("unterminated object".to_string(), start));
        }
        match bytes[i] {
            b',' => i = skip_ws(bytes, i + 1),
            b'}' => return Ok((Parsed::Object(fields), i + 1)),
            b => return Err((format!("expected `,` or `}}` in object, got {b:#x}"), i)),
        }
    }
}

/// Parse an object key.  In `Dialect::Strict` the key must be a
/// quoted JSON string.  In `Dialect::Lenient` a leading
/// ASCII-letter or `_` additionally opens a bare identifier
/// that continues while the next byte is alphanumeric or `_` —
/// matching the loft identifier grammar used by the legacy
/// `vector<T>.parse(text)` path.
fn parse_object_key(
    bytes: &[u8],
    i: usize,
    dialect: Dialect,
) -> Result<(String, usize), (String, usize)> {
    if i < bytes.len() && bytes[i] == b'"' {
        let (key, j) = parse_string(bytes, i)?;
        match key {
            Parsed::Str(s) => Ok((s, j)),
            _ => unreachable!("parse_string always returns Parsed::Str"),
        }
    } else if dialect == Dialect::Lenient
        && i < bytes.len()
        && (bytes[i].is_ascii_alphabetic() || bytes[i] == b'_')
    {
        let start = i;
        let mut j = i + 1;
        while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
            j += 1;
        }
        // Safe: all bytes accepted above are ASCII.
        let name = std::str::from_utf8(&bytes[start..j])
            .expect("ASCII identifier slice")
            .to_string();
        Ok((name, j))
    } else {
        Err(("expected string key in object".to_string(), i))
    }
}

fn skip_ws(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\n' | b'\r' => i += 1,
            _ => break,
        }
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitives() {
        assert!(matches!(parse("null").unwrap(), Parsed::Null));
        assert!(matches!(parse("true").unwrap(), Parsed::Bool(true)));
        assert!(matches!(parse("false").unwrap(), Parsed::Bool(false)));
    }

    #[test]
    fn numbers() {
        assert!(matches!(parse("0").unwrap(), Parsed::Number(v) if (v - 0.0).abs() < f64::EPSILON));
        assert!(
            matches!(parse("42").unwrap(), Parsed::Number(v) if (v - 42.0).abs() < f64::EPSILON)
        );
        assert!(
            matches!(parse("-17.5").unwrap(), Parsed::Number(v) if (v - (-17.5)).abs() < f64::EPSILON)
        );
        assert!(
            matches!(parse("1.5e3").unwrap(), Parsed::Number(v) if (v - 1500.0).abs() < f64::EPSILON)
        );
    }

    #[test]
    fn strings() {
        let got = parse(r#""hello""#).unwrap();
        assert!(matches!(got, Parsed::Str(ref s) if s == "hello"));
        let got = parse(r#""\"quote\"""#).unwrap();
        assert!(matches!(got, Parsed::Str(ref s) if s == "\"quote\""));
        let got = parse(r#""line\nfeed""#).unwrap();
        assert!(matches!(got, Parsed::Str(ref s) if s == "line\nfeed"));
    }

    #[test]
    fn whitespace_tolerated() {
        assert!(matches!(parse("  null  ").unwrap(), Parsed::Null));
    }

    #[test]
    fn arrays() {
        assert!(matches!(parse("[]").unwrap(), Parsed::Array(ref v) if v.is_empty()));
        let got = parse("[1, 2, 3]").unwrap();
        let Parsed::Array(v) = got else {
            panic!("expected array");
        };
        assert_eq!(v.len(), 3);
        assert!(matches!(v[0], Parsed::Number(n) if (n - 1.0).abs() < f64::EPSILON));
        let nested = parse("[[1], [2, 3]]").unwrap();
        let Parsed::Array(outer) = nested else {
            panic!("expected array");
        };
        assert_eq!(outer.len(), 2);
    }

    #[test]
    fn objects() {
        assert!(matches!(parse("{}").unwrap(), Parsed::Object(ref v) if v.is_empty()));
        let got = parse(r#"{"a": 1, "b": "hi"}"#).unwrap();
        let Parsed::Object(fields) = got else {
            panic!("expected object");
        };
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].0, "a");
        assert!(matches!(fields[0].2, Parsed::Number(n) if (n - 1.0).abs() < f64::EPSILON));
        assert_eq!(fields[1].0, "b");
        assert!(matches!(fields[1].2, Parsed::Str(ref s) if s == "hi"));
    }

    #[test]
    fn nested_mixed() {
        let got = parse(r#"{"items": [1, {"x": true}], "n": null}"#).unwrap();
        let Parsed::Object(fields) = got else {
            panic!("expected object");
        };
        assert_eq!(fields.len(), 2);
        let Parsed::Array(items) = &fields[0].2 else {
            panic!("expected array");
        };
        assert_eq!(items.len(), 2);
        let Parsed::Object(inner) = &items[1] else {
            panic!("expected inner object");
        };
        assert_eq!(inner[0].0, "x");
        assert!(matches!(inner[0].2, Parsed::Bool(true)));
    }

    // ── Q1: structured errors with path / line:col / snippet ────────

    #[test]
    fn err_root_failure_has_empty_path() {
        let err = parse("xyz").unwrap_err();
        assert_eq!(err.path, "");
        assert_eq!(err.byte_offset, 0);
    }

    #[test]
    fn err_inside_array_carries_index_path() {
        let err = parse("[1, 2, 1.]").unwrap_err();
        assert_eq!(err.path, "/2");
    }

    #[test]
    fn err_inside_object_carries_field_path() {
        let err = parse(r#"{"a": 1, "b": 1.}"#).unwrap_err();
        assert_eq!(err.path, "/b");
    }

    #[test]
    fn err_nested_path_is_full_pointer() {
        let err = parse(r#"{"users": [{"name": "x"}, {"name": 1.}]}"#).unwrap_err();
        assert_eq!(err.path, "/users/1/name");
    }

    #[test]
    fn err_path_escapes_slash_and_tilde() {
        // Field "a/b~c" → "/a~1b~0c" per RFC 6901.
        let err = parse(r#"{"a/b~c": 1.}"#).unwrap_err();
        assert_eq!(err.path, "/a~1b~0c");
    }

    #[test]
    fn line_col_basic() {
        assert_eq!(line_col_of("abc", 0), (1, 1));
        assert_eq!(line_col_of("abc", 2), (1, 3));
        assert_eq!(line_col_of("a\nbc", 2), (2, 1));
        assert_eq!(line_col_of("a\nbc", 3), (2, 2));
        assert_eq!(line_col_of("a\nb\nc", 4), (3, 1));
    }

    #[test]
    fn format_error_includes_path_line_col_and_caret() {
        let raw = "{\n  \"x\": 1.\n}";
        let err = parse(raw).unwrap_err();
        let formatted = format_error(raw, &err, 1, 1);
        // Diagnostic mentions path, line, col, message, and a caret.
        assert!(formatted.contains("/x"), "missing path: {formatted}");
        assert!(
            formatted.contains("line 2"),
            "missing line number: {formatted}"
        );
        assert!(formatted.contains('^'), "missing caret: {formatted}");
        assert!(
            formatted.contains("expected digit after `.`"),
            "missing message: {formatted}"
        );
    }

    #[test]
    fn format_error_root_path_renders_as_root_label() {
        let formatted = format_error("xyz", &parse("xyz").unwrap_err(), 0, 0);
        assert!(
            formatted.contains("(root)"),
            "root path label missing: {formatted}"
        );
    }

    #[test]
    fn malformed_collections() {
        assert!(parse("[").is_err());
        assert!(parse("[1,]").is_err());
        assert!(parse("{").is_err());
        assert!(parse(r#"{"a"}"#).is_err());
        assert!(parse(r#"{"a": 1,}"#).is_err());
        assert!(parse(r"{a: 1}").is_err());
    }

    #[test]
    fn malformed_returns_err() {
        assert!(parse("").is_err());
        assert!(parse("nu").is_err());
        assert!(parse("1.").is_err());
        assert!(parse(r#""no-close"#).is_err());
        assert!(parse("null trailing").is_err());
    }

    // ── Dialect::Lenient accepts loft bare-identifier keys ──────

    #[test]
    fn parse_with_strict_rejects_bare_key() {
        assert!(parse_with(r"{a: 1}", Dialect::Strict).is_err());
        assert!(parse_with(r"{x_1: null}", Dialect::Strict).is_err());
    }

    #[test]
    fn parse_with_lenient_accepts_bare_key() {
        let Parsed::Object(fields) = parse_with(r"{val: 7}", Dialect::Lenient).unwrap() else {
            panic!("expected object");
        };
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].0, "val");
        assert!(matches!(fields[0].2, Parsed::Number(n) if (n - 7.0).abs() < f64::EPSILON));
    }

    #[test]
    fn parse_with_lenient_allows_mixed_quoted_and_bare() {
        let Parsed::Object(fields) =
            parse_with(r#"{a: 1, "b": 2, c_2: 3}"#, Dialect::Lenient).unwrap()
        else {
            panic!("expected object");
        };
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].0, "a");
        assert_eq!(fields[1].0, "b");
        assert_eq!(fields[2].0, "c_2");
    }

    #[test]
    fn parse_with_lenient_rejects_non_identifier_keys() {
        // Numeric object keys are not accepted even under Lenient —
        // only `[A-Za-z_][A-Za-z0-9_]*` identifiers or quoted
        // strings qualify as keys.  Bare-identifier *values* are
        // accepted separately (see `parse_with_lenient_accepts_bare_ident_value`).
        assert!(parse_with(r"{1: 2}", Dialect::Lenient).is_err());
        assert!(parse_with(r"{-foo: 1}", Dialect::Lenient).is_err());
    }

    #[test]
    fn parse_default_is_strict() {
        // Default Dialect is Strict — behaviour identical to bare `parse`.
        assert!(Dialect::default() == Dialect::Strict);
    }

    // ── Dialect::Lenient also accepts bare identifier values ──

    #[test]
    fn parse_with_lenient_accepts_bare_ident_value() {
        // `Daily` here represents a loft enum tag in value position.
        let Parsed::Object(fields) = parse_with(r"{category: Daily}", Dialect::Lenient).unwrap()
        else {
            panic!("expected object");
        };
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].0, "category");
        assert!(
            matches!(&fields[0].2, Parsed::Ident(s) if s == "Daily"),
            "expected Ident(\"Daily\"), got {:?}",
            fields[0].2
        );
    }

    #[test]
    fn parse_with_lenient_recognises_true_false_null_as_bare() {
        let Parsed::Object(fields) =
            parse_with(r"{a: true, b: false, c: null}", Dialect::Lenient).unwrap()
        else {
            panic!("expected object");
        };
        assert!(matches!(fields[0].2, Parsed::Bool(true)));
        assert!(matches!(fields[1].2, Parsed::Bool(false)));
        assert!(matches!(fields[2].2, Parsed::Null));
    }

    #[test]
    fn parse_with_strict_still_rejects_bare_ident_value() {
        assert!(parse_with(r"{category: Daily}", Dialect::Strict).is_err());
        assert!(parse_with(r"{x: hello}", Dialect::Strict).is_err());
    }

    #[test]
    fn parse_with_lenient_top_level_bare_ident() {
        // Not only in object values — a bare identifier is a valid
        // top-level loft literal (e.g. a single enum tag stored as
        // the whole record).
        let parsed = parse_with("Hourly", Dialect::Lenient).unwrap();
        assert!(
            matches!(&parsed, Parsed::Ident(s) if s == "Hourly"),
            "expected Ident(\"Hourly\"), got {parsed:?}",
        );
    }

    #[test]
    fn parse_with_lenient_bare_ident_in_array() {
        let Parsed::Array(items) =
            parse_with(r"[Daily, Weekly, Hourly]", Dialect::Lenient).unwrap()
        else {
            panic!("expected array");
        };
        assert_eq!(items.len(), 3);
        assert!(matches!(&items[0], Parsed::Ident(s) if s == "Daily"));
        assert!(matches!(&items[1], Parsed::Ident(s) if s == "Weekly"));
        assert!(matches!(&items[2], Parsed::Ident(s) if s == "Hourly"));
    }

    #[test]
    fn parse_with_lenient_mixed_example_from_data_structures_test() {
        // This input comes from tests/data_structures.rs::record —
        // the legacy parser's canonical round-trip shape.
        let input = r#"{ name: "Hello World!", category: Hourly, size: 12345, percentage: 0.15 }"#;
        let Parsed::Object(fields) = parse_with(input, Dialect::Lenient).unwrap() else {
            panic!("expected object");
        };
        assert_eq!(fields.len(), 4);
        assert!(matches!(&fields[0].2, Parsed::Str(s) if s == "Hello World!"));
        assert!(matches!(&fields[1].2, Parsed::Ident(s) if s == "Hourly"));
        assert!(matches!(fields[2].2, Parsed::Number(n) if (n - 12345.0).abs() < f64::EPSILON));
        assert!(matches!(fields[3].2, Parsed::Number(n) if (n - 0.15).abs() < 1e-9));
    }
}
